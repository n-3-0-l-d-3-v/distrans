//! The reliable, ordered, exactly-once-delivered connection state
//! machine (ticket 003). Poll-driven, matching `channel::Channel`'s own
//! style: nothing here reads physical time, or even virtual time on its
//! own — every method takes the caller's current `Tick` and returns the
//! raw datagrams (already `frame::encode`d) that should now be handed to
//! whatever's carrying them (in tests and this phase, a `channel::Channel`).
//!
//! See `docs/design/decisions/ADR-003-reliable-transport.md` for the
//! design: no sequence-number consumption by SYN/FIN (a deliberate
//! divergence from TCP, discussed there), a single connection-level RTO
//! timer keyed off the oldest unacknowledged byte, and Karn's algorithm
//! for RTT sampling.

use std::collections::{BTreeMap, VecDeque};

pub use channel::Tick;
use frame::Frame;

use crate::rto::RtoEstimator;
use crate::segment::{SackRange, SegmentHeader, MAX_SACK_RANGES};

const FLAG_SYN: u16 = 1;
const FLAG_ACK: u16 = 2;
const FLAG_FIN: u16 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    /// A control segment (handshake or FIN) or a data segment exceeded
    /// `Config::max_retries` without being acknowledged.
    TooManyRetries,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Listening,
    SynSent,
    SynReceived,
    Established,
    /// `close()` has been called; still finishing outstanding data and/or
    /// the FIN handshake.
    Closing,
    Closed,
    Failed(FailureReason),
}

#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// Maximum application bytes per data segment.
    pub segment_size: usize,
    /// Maximum number of data segments outstanding (sent, unacknowledged)
    /// at once — a fixed stand-in for real flow/congestion control until
    /// ticket 004 replaces it with a real advertised/congestion window.
    pub max_in_flight_segments: usize,
    pub min_rto: u64,
    pub max_rto: u64,
    /// A segment or control message failing this many consecutive
    /// (re)transmissions fails the connection.
    pub max_retries: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            segment_size: 512,
            max_in_flight_segments: 8,
            min_rto: 2,
            max_rto: 500,
            max_retries: 12,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub data_segments_sent: u64,
    pub data_segments_retransmitted: u64,
    pub ack_segments_sent: u64,
    pub control_retransmissions: u64,
    pub bytes_sent_app: u64,
    pub bytes_delivered_app: u64,
    pub duplicate_bytes_received: u64,
}

#[derive(Debug, Clone)]
struct PendingControl {
    sent_at: Tick,
    attempts: u32,
}

#[derive(Debug, Clone)]
struct UnackedSegment {
    data: Vec<u8>,
    sent_at: Tick,
    retransmit_count: u32,
    /// The peer's most recent ACK reported this range as received
    /// out-of-order (SACK) — skip it on the next timeout-driven
    /// retransmission sweep, even though it isn't cumulatively acked yet.
    sacked: bool,
}

pub struct Connection {
    config: Config,
    state: State,
    now: Tick,
    rto: RtoEstimator,
    stats: Stats,

    handshake: Option<PendingControl>,

    // Send side.
    send_queue: VecDeque<u8>,
    next_seq: u32,
    send_base: u32,
    unacked: BTreeMap<u32, UnackedSegment>,
    close_requested: bool,
    fin_pending: Option<PendingControl>,
    fin_acked: bool,

    // Receive side.
    expected_seq: u32,
    recv_buffer: BTreeMap<u32, Vec<u8>>,
    delivered: VecDeque<u8>,
    peer_fin_seen: bool,
}

fn encode_frame(sequence: u32, flags: u16, payload: Vec<u8>) -> Vec<u8> {
    frame::encode(&Frame {
        frame_type: 0,
        sequence,
        flags,
        payload,
    })
}

impl Connection {
    fn blank(config: Config, now: Tick, state: State) -> Self {
        Self {
            config,
            state,
            now,
            rto: RtoEstimator::new(config.min_rto, config.max_rto),
            stats: Stats::default(),
            handshake: None,
            send_queue: VecDeque::new(),
            next_seq: 0,
            send_base: 0,
            unacked: BTreeMap::new(),
            close_requested: false,
            fin_pending: None,
            fin_acked: false,
            expected_seq: 0,
            recv_buffer: BTreeMap::new(),
            delivered: VecDeque::new(),
            peer_fin_seen: false,
        }
    }

    /// Begins an active (client) connection: returns the connection plus
    /// the initial SYN datagram to send.
    pub fn connect(config: Config, now: Tick) -> (Self, Vec<u8>) {
        let mut c = Self::blank(config, now, State::SynSent);
        c.handshake = Some(PendingControl {
            sent_at: now,
            attempts: 1,
        });
        let syn = c.build_syn();
        (c, syn)
    }

    /// Begins a passive (server) connection, waiting for a SYN.
    pub fn listen(config: Config, now: Tick) -> Self {
        Self::blank(config, now, State::Listening)
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.state, State::Closed)
    }

    pub fn failure(&self) -> Option<FailureReason> {
        match self.state {
            State::Failed(r) => Some(r),
            _ => None,
        }
    }

    /// Whether the peer has sent (and we've seen) its FIN — the
    /// application-level signal that "the peer is done sending," on
    /// which the application decides whether/when to close its own side.
    /// Mirrors a real socket API's read-returns-EOF: this crate never
    /// closes a connection on the caller's behalf.
    pub fn peer_closed(&self) -> bool {
        self.peer_fin_seen
    }

    // ---- Frame builders --------------------------------------------

    fn header_bytes(&self) -> Vec<u8> {
        let h = SegmentHeader {
            ack: self.expected_seq,
            fin_ack: false,
            sack: self.current_sack_ranges(),
        };
        let mut b = Vec::new();
        h.encode(&mut b);
        b
    }

    fn current_sack_ranges(&self) -> Vec<SackRange> {
        let mut ranges = Vec::new();
        let mut iter = self.recv_buffer.iter().peekable();
        while let Some((&start, data)) = iter.next() {
            let mut end = start + data.len() as u32 - 1;
            while let Some(&(&next_start, next_data)) = iter.peek() {
                if next_start == end + 1 {
                    end = next_start + next_data.len() as u32 - 1;
                    iter.next();
                } else {
                    break;
                }
            }
            ranges.push(SackRange { start, end });
            if ranges.len() == MAX_SACK_RANGES {
                break;
            }
        }
        ranges
    }

    fn build_syn(&self) -> Vec<u8> {
        encode_frame(0, FLAG_SYN, Vec::new())
    }

    fn build_synack(&self) -> Vec<u8> {
        encode_frame(0, FLAG_SYN | FLAG_ACK, Vec::new())
    }

    fn build_handshake_ack(&self) -> Vec<u8> {
        encode_frame(0, FLAG_ACK, Vec::new())
    }

    fn build_fin(&self) -> Vec<u8> {
        // FIN carries the normal header (cumulative ack/sack) piggybacked,
        // so a lost pure-ACK doesn't regress the peer's view of what
        // we've received.
        let payload = self.header_bytes();
        encode_frame(self.send_base, FLAG_FIN | FLAG_ACK, payload)
    }

    fn build_pure_ack(&self, fin_ack: bool) -> Vec<u8> {
        let h = SegmentHeader {
            ack: self.expected_seq,
            fin_ack,
            sack: self.current_sack_ranges(),
        };
        let mut payload = Vec::new();
        h.encode(&mut payload);
        encode_frame(self.send_base, FLAG_ACK, payload)
    }

    fn build_data_frame(seq: u32, data: &[u8], header_bytes: &[u8]) -> Vec<u8> {
        let mut payload = Vec::with_capacity(header_bytes.len() + data.len());
        payload.extend_from_slice(header_bytes);
        payload.extend_from_slice(data);
        encode_frame(seq, FLAG_ACK, payload)
    }

    // ---- Sending ------------------------------------------------------

    /// Queues `data` for reliable delivery, returning whatever can be
    /// sent immediately (subject to `max_in_flight_segments`).
    pub fn send(&mut self, now: Tick, data: &[u8]) -> Vec<Vec<u8>> {
        self.now = self.now.max(now);
        self.send_queue.extend(data.iter().copied());
        self.stats.bytes_sent_app += data.len() as u64;
        self.drain_and_maybe_fin()
    }

    fn try_send_more(&mut self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        if !matches!(self.state, State::Established | State::Closing) {
            return out;
        }
        while !self.send_queue.is_empty() && self.unacked.len() < self.config.max_in_flight_segments
        {
            let chunk_len = self.config.segment_size.min(self.send_queue.len());
            let data: Vec<u8> = self.send_queue.drain(..chunk_len).collect();
            let seq = self.next_seq;
            self.next_seq += chunk_len as u32;
            let header = self.header_bytes();
            out.push(Self::build_data_frame(seq, &data, &header));
            self.unacked.insert(
                seq,
                UnackedSegment {
                    data,
                    sent_at: self.now,
                    retransmit_count: 0,
                    sacked: false,
                },
            );
            self.stats.data_segments_sent += 1;
        }
        out
    }

    fn maybe_send_fin(&mut self) -> Option<Vec<u8>> {
        if self.close_requested
            && self.fin_pending.is_none()
            && !self.fin_acked
            && self.state == State::Established
            && self.send_queue.is_empty()
            && self.unacked.is_empty()
        {
            self.state = State::Closing;
            self.fin_pending = Some(PendingControl {
                sent_at: self.now,
                attempts: 1,
            });
            Some(self.build_fin())
        } else {
            None
        }
    }

    fn drain_and_maybe_fin(&mut self) -> Vec<Vec<u8>> {
        let mut out = self.try_send_more();
        out.extend(self.maybe_send_fin());
        self.check_fully_closed();
        out
    }

    /// Begins a graceful close: once all queued/outstanding data is
    /// acknowledged, a FIN is sent (and retried) until the peer
    /// acknowledges it. The connection is fully `Closed` once our FIN is
    /// acked *and* the peer's own FIN has been seen.
    pub fn close(&mut self, now: Tick) -> Vec<Vec<u8>> {
        self.now = self.now.max(now);
        self.close_requested = true;
        self.drain_and_maybe_fin()
    }

    fn check_fully_closed(&mut self) {
        if self.state == State::Closing && self.fin_acked && self.peer_fin_seen {
            self.state = State::Closed;
        }
    }

    /// Newly available in-order application bytes.
    pub fn recv(&mut self) -> Vec<u8> {
        self.delivered.drain(..).collect()
    }

    // ---- Receiving ------------------------------------------------------

    pub fn on_datagram(&mut self, now: Tick, datagram: &[u8]) -> Vec<Vec<u8>> {
        self.now = self.now.max(now);
        let mut out = Vec::new();

        let Ok(f) = frame::decode(datagram) else {
            return out; // corrupted: indistinguishable from lost, handled by retransmission
        };
        // A bare SYN/SYN-ACK carries a genuinely empty frame payload (no
        // transport header at all, since the handshake predates any
        // ack/sack state worth reporting); everything else's payload
        // starts with a `SegmentHeader`.
        let (header, app_payload): (SegmentHeader, &[u8]) = if f.payload.is_empty() {
            (SegmentHeader::default(), &[])
        } else {
            match SegmentHeader::decode(&f.payload) {
                Some(v) => v,
                None => return out, // malformed transport header: drop, like a corrupted datagram
            }
        };

        let is_syn = f.flags & FLAG_SYN != 0;
        let is_ack = f.flags & FLAG_ACK != 0;
        let is_fin = f.flags & FLAG_FIN != 0;

        // A fully `Closed` connection still answers a lingering,
        // retransmitted FIN with a fresh fin_ack. Without this, a race
        // where our own fin_ack (sent right before we ourselves reached
        // `Closed`) is lost leaves the peer retransmitting its FIN
        // forever against a side that has stopped listening for
        // anything — exactly the failure mode real TCP's TIME_WAIT state
        // exists to prevent. This is a narrower fix than a full
        // TIME_WAIT (no timer, no separate state): it only helps for as
        // long as this `Connection` value is still alive and fed
        // datagrams, which is the case in this repo's tests and its
        // ticket 006 workload; see ADR-003 for the honest limitation.
        if self.state == State::Closed && is_fin {
            return vec![self.build_pure_ack(true)];
        }

        match self.state {
            State::Listening if is_syn && !is_ack => {
                self.state = State::SynReceived;
                self.handshake = Some(PendingControl {
                    sent_at: self.now,
                    attempts: 1,
                });
                out.push(self.build_synack());
                return out;
            }
            State::SynReceived if is_syn && !is_ack => {
                // Our SYN-ACK likely never arrived; resend without
                // bumping our own attempt/backoff counters — this retry
                // was the peer's doing, not our timer's.
                out.push(self.build_synack());
                return out;
            }
            State::SynSent if is_syn && is_ack => {
                if let Some(h) = self.handshake.take() {
                    if h.attempts == 1 {
                        self.rto.on_sample(self.now - h.sent_at);
                    }
                }
                self.state = State::Established;
                out.push(self.build_handshake_ack());
                // Deliberately falls through to the shared
                // Established/Closing block below rather than returning:
                // a real (or, more commonly here, a hostile-channel
                // scripted) SYN-ACK can arrive more than once, and each
                // one must re-send our completing ACK — see the
                // `State::Established | State::Closing` catch-all arm
                // just below for why a *duplicate* SYN-ACK, received
                // after we've already moved on, also needs this.
            }
            State::SynReceived if is_ack && !is_syn => {
                // Any ACK-flagged segment completes the handshake here —
                // not just a bare, dataless one. A real client typically
                // moves to Established and starts sending data
                // immediately, and that data segment (which also carries
                // FLAG_ACK) is this side's only signal that the handshake
                // finished; requiring a separate, empty completing ACK
                // first would mean a single lost bare ACK strands the
                // server in SynReceived forever even though the client
                // believes everything is fine (found via
                // `arbitrary_data_over_an_arbitrary_hostile_profile_arrives_intact`,
                // see ADR-003).
                if let Some(h) = self.handshake.take() {
                    if h.attempts == 1 {
                        self.rto.on_sample(self.now - h.sent_at);
                    }
                }
                self.state = State::Established;
                // Falls through: this same datagram's ack/data/fin must
                // still be processed below.
            }
            State::Established | State::Closing if is_syn && is_ack => {
                // A duplicate SYN-ACK: our own first completing ACK was
                // presumably lost, and the peer is still retrying its
                // handshake. Resend the completing ACK (idempotent) so
                // the peer isn't left retrying forever against a side
                // that already moved on and will otherwise never repeat
                // it (this exact scenario is what originally motivated
                // the fallthrough above, before it was generalized).
                out.push(self.build_handshake_ack());
            }
            _ => {}
        }

        if self.state == State::Established || self.state == State::Closing {
            if is_ack {
                self.apply_ack(&header);
            }
            let mut should_ack = false;
            if !app_payload.is_empty() {
                self.deliver_data(f.sequence, app_payload);
                should_ack = true;
            }
            if is_fin {
                self.peer_fin_seen = true;
                should_ack = true;
            }
            if should_ack {
                self.stats.ack_segments_sent += 1;
                out.push(self.build_pure_ack(is_fin));
            }
            out.extend(self.drain_and_maybe_fin());
        }

        out
    }

    fn apply_ack(&mut self, header: &SegmentHeader) {
        if header.ack > self.send_base {
            let newly_acked: Vec<u32> = self
                .unacked
                .range(..header.ack)
                .map(|(&seq, _)| seq)
                .collect();
            for seq in newly_acked {
                if let Some(seg) = self.unacked.remove(&seq) {
                    if seg.retransmit_count == 0 {
                        self.rto.on_sample(self.now - seg.sent_at);
                    }
                }
            }
            self.send_base = header.ack;
        }
        for r in &header.sack {
            for (_, seg) in self.unacked.range_mut(r.start..=r.end) {
                seg.sacked = true;
            }
        }
        if header.fin_ack {
            self.fin_acked = true;
            self.fin_pending = None;
            self.check_fully_closed();
        }
    }

    fn deliver_data(&mut self, seq: u32, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let end = seq + data.len() as u32;
        if end <= self.expected_seq {
            self.stats.duplicate_bytes_received += data.len() as u64;
            return;
        }
        if seq <= self.expected_seq {
            let skip = (self.expected_seq - seq) as usize;
            self.stats.duplicate_bytes_received += skip as u64;
            self.delivered.extend(&data[skip..]);
            self.stats.bytes_delivered_app += (data.len() - skip) as u64;
            self.expected_seq = end;
            while let Some((&k, _)) = self.recv_buffer.range(self.expected_seq..).next() {
                if k != self.expected_seq {
                    break;
                }
                let v = self.recv_buffer.remove(&k).unwrap();
                self.expected_seq += v.len() as u32;
                self.stats.bytes_delivered_app += v.len() as u64;
                self.delivered.extend(v);
            }
        } else {
            self.recv_buffer.insert(seq, data.to_vec());
        }
    }

    // ---- Timers ------------------------------------------------------

    pub fn on_tick(&mut self, now: Tick) -> Vec<Vec<u8>> {
        self.now = self.now.max(now);
        let mut out = Vec::new();

        match self.state {
            State::SynSent | State::SynReceived => {
                if let Some(h) = &mut self.handshake {
                    if (self.now - h.sent_at) >= self.rto.current() {
                        h.attempts += 1;
                        if h.attempts > self.config.max_retries {
                            self.state = State::Failed(FailureReason::TooManyRetries);
                            self.handshake = None;
                            return out;
                        }
                        self.rto.on_timeout();
                        h.sent_at = self.now;
                        self.stats.control_retransmissions += 1;
                        out.push(match self.state {
                            State::SynSent => self.build_syn(),
                            State::SynReceived => self.build_synack(),
                            _ => unreachable!(),
                        });
                    }
                }
            }
            State::Established | State::Closing => {
                let oldest = self.unacked.iter().next().map(|(&seq, s)| (seq, s.sent_at));
                if let Some((_, oldest_sent_at)) = oldest {
                    if (self.now - oldest_sent_at) >= self.rto.current() {
                        self.rto.on_timeout();
                        let header = self.header_bytes();
                        let mut failed = false;
                        for (&seq, seg) in self.unacked.iter_mut() {
                            if seg.sacked {
                                continue;
                            }
                            seg.retransmit_count += 1;
                            seg.sent_at = self.now;
                            self.stats.data_segments_retransmitted += 1;
                            if seg.retransmit_count > self.config.max_retries {
                                failed = true;
                            }
                            out.push(Self::build_data_frame(seq, &seg.data, &header));
                        }
                        if failed {
                            self.state = State::Failed(FailureReason::TooManyRetries);
                            self.unacked.clear();
                            return out;
                        }
                    }
                }
                if let Some(f) = &mut self.fin_pending {
                    if (self.now - f.sent_at) >= self.rto.current() {
                        f.attempts += 1;
                        if f.attempts > self.config.max_retries {
                            self.state = State::Failed(FailureReason::TooManyRetries);
                            self.fin_pending = None;
                            return out;
                        }
                        self.rto.on_timeout();
                        f.sent_at = self.now;
                        self.stats.control_retransmissions += 1;
                        out.push(self.build_fin());
                    }
                }
                out.extend(self.drain_and_maybe_fin());
            }
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config {
            segment_size: 16,
            max_in_flight_segments: 4,
            min_rto: 5,
            max_rto: 100,
            max_retries: 5,
        }
    }

    /// The completing ACK of the 3-way handshake is never delivered
    /// (simulating it being lost); the client instead immediately sends
    /// real data, whose segment also carries the ACK flag. The server
    /// must still complete its handshake and deliver the data — it must
    /// not require a separate, bare completing ACK first.
    #[test]
    fn a_lost_bare_completing_ack_does_not_strand_the_server_if_data_follows() {
        let (mut client, syn) = Connection::connect(cfg(), Tick(0));
        let mut server = Connection::listen(cfg(), Tick(0));

        let synack = server.on_datagram(Tick(1), &syn);
        assert_eq!(synack.len(), 1);
        assert_eq!(server.state(), State::SynReceived);

        let client_responses = client.on_datagram(Tick(2), &synack[0]);
        assert_eq!(client.state(), State::Established);
        // client_responses[0] is the bare completing ACK — simulate it
        // being lost by simply never delivering it to the server.
        assert_eq!(client_responses.len(), 1);

        let data_segments = client.send(Tick(3), b"hello, server");
        assert_eq!(data_segments.len(), 1);

        // The server never saw the bare ACK, only this data segment.
        server.on_datagram(Tick(4), &data_segments[0]);
        assert_eq!(
            server.state(),
            State::Established,
            "server must complete its handshake from a data-carrying ACK alone"
        );
        assert_eq!(server.recv(), b"hello, server");
    }

    /// The mirror case: the server's SYN-ACK is retransmitted (because it
    /// never saw the client's first completing ACK) after the client has
    /// already moved on to `Established`. The client must resend its
    /// completing ACK again, not silently treat the duplicate as routine.
    #[test]
    fn a_duplicate_synack_after_establishment_gets_the_completing_ack_resent() {
        let (mut client, syn) = Connection::connect(cfg(), Tick(0));
        let mut server = Connection::listen(cfg(), Tick(0));
        let synack = server.on_datagram(Tick(1), &syn);
        client.on_datagram(Tick(2), &synack[0]);
        assert_eq!(client.state(), State::Established);

        // Server retransmits its SYN-ACK (its own completing ACK never
        // arrived, from its point of view).
        let resent = client.on_datagram(Tick(3), &synack[0]);
        assert_eq!(
            resent.len(),
            1,
            "a duplicate SYN-ACK after establishment must still get a completing ACK back"
        );
        let ack_frame = frame::decode(&resent[0]).unwrap();
        assert_eq!(ack_frame.flags & FLAG_ACK, FLAG_ACK);
        assert_eq!(ack_frame.flags & FLAG_SYN, 0);
    }

    /// A FIN that arrives after this side is already fully `Closed`
    /// (because its own fin_ack to the peer was lost) still gets a fresh
    /// fin_ack — otherwise the peer retries forever against a side that
    /// has stopped listening.
    #[test]
    fn a_closed_connection_still_acks_a_lingering_retransmitted_fin() {
        let (mut client, syn) = Connection::connect(cfg(), Tick(0));
        let mut server = Connection::listen(cfg(), Tick(0));
        let synack = server.on_datagram(Tick(1), &syn);
        let ack = client.on_datagram(Tick(2), &synack[0]);
        server.on_datagram(Tick(3), &ack[0]);
        assert_eq!(server.state(), State::Established);

        let fin = client.close(Tick(4));
        assert_eq!(fin.len(), 1);
        let fin_ack = server.on_datagram(Tick(5), &fin[0]);
        assert_eq!(fin_ack.len(), 1);
        // Simulate the fin_ack being lost: the client never sees it, and
        // the server independently closes its own side too.
        let server_fin = server.close(Tick(6));
        assert_eq!(server_fin.len(), 1);
        let client_fin_ack = client.on_datagram(Tick(7), &server_fin[0]);
        server.on_datagram(Tick(8), &client_fin_ack[0]);
        assert_eq!(server.state(), State::Closed);

        // The client, having never seen its fin_ack, retransmits its FIN
        // once more. The server must still answer even though it is
        // already Closed.
        let retransmitted_fin = client.build_fin();
        let response = server.on_datagram(Tick(9), &retransmitted_fin);
        assert_eq!(response.len(), 1);
        let (header, _) =
            SegmentHeader::decode(&frame::decode(&response[0]).unwrap().payload).unwrap();
        assert!(header.fin_ack);
    }
}
