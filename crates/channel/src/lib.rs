//! THE WIRE's foundation: a simulated datagram channel that guarantees
//! nothing, per `docs/design/WIRE.md`. Time is virtual (`Tick`, advanced
//! only by `Channel::advance`/`Channel::deliver_ready`), and every fault
//! it injects (loss, duplication, reordering, corruption, truncation)
//! comes from a seeded `SplitMix64`, so a whole run replays exactly from
//! `(seed, profile)`. A `Script` can override the random profile with
//! explicit, deterministic per-datagram rules for regression tests of one
//! specific corner case.
//!
//! A `Channel` is one direction. A duplex link between two endpoints is
//! two independent `Channel`s, each with its own profile — real hostile
//! links are rarely symmetric.

mod fault;
mod rng;
mod tick;

pub use fault::{FaultProfile, FaultStats, Script, ScriptedFault};
pub use tick::Tick;

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use rng::SplitMix64;

/// A datagram in flight: raw bytes plus the tick it becomes deliverable.
#[derive(Debug, Clone, PartialEq, Eq)]
struct InFlight {
    arrives: Tick,
    /// Insertion order, used only to break `arrives` ties FIFO so two
    /// datagrams that land on the same tick don't reorder for no reason.
    seq: u64,
    payload: Vec<u8>,
}

/// One direction of a hostile link. `send` queues a datagram (applying
/// faults immediately, at send time, using virtual time only to decide
/// *when* it becomes deliverable); `advance`/`deliver_ready` move virtual
/// time forward and hand back whatever has arrived.
pub struct Channel {
    profile: FaultProfile,
    script: Script,
    rng: SplitMix64,
    now: Tick,
    /// Min-heap by (arrival tick, insertion order).
    in_flight: BinaryHeap<Reverse<(Tick, u64, usize)>>,
    /// Payloads, indexed by the same `usize` id used in `in_flight`. Slots
    /// are never reused within one `Channel`'s lifetime, so ids stay valid.
    payloads: Vec<Vec<u8>>,
    next_datagram_index: u64,
    stats: FaultStats,
}

impl Channel {
    /// A channel with a random fault profile only (no scripted overrides).
    pub fn new(seed: u64, profile: FaultProfile) -> Self {
        Self::with_script(seed, profile, Script::new())
    }

    /// A channel with both a random profile and scripted overrides. A
    /// scripted rule for a given datagram index takes precedence over
    /// (is applied in addition to, per each rule's own semantics — see
    /// `send`) the random profile's own draws for that datagram: this
    /// lets a test hold most of the channel's behavior random while
    /// pinning one specific datagram's fate exactly.
    pub fn with_script(seed: u64, profile: FaultProfile, script: Script) -> Self {
        profile.validate().expect("invalid FaultProfile");
        Self {
            profile,
            script,
            rng: SplitMix64::new(seed),
            now: Tick::ZERO,
            in_flight: BinaryHeap::new(),
            payloads: Vec::new(),
            next_datagram_index: 0,
            stats: FaultStats::default(),
        }
    }

    pub fn now(&self) -> Tick {
        self.now
    }

    pub fn stats(&self) -> FaultStats {
        self.stats
    }

    /// Sends one datagram. Faults are decided now, at send time — loss,
    /// duplication, corruption and truncation are properties of *this*
    /// send, not of delivery — but the resulting datagram(s) don't become
    /// visible to `deliver_ready` until virtual time reaches their
    /// `arrives` tick.
    ///
    /// A scripted `Drop`/`Duplicate`/`Delay`/`Corrupt`/`Truncate` rule for
    /// this datagram's index is applied *instead of* that fault's random
    /// draw (the other faults still draw normally) — e.g. a scripted
    /// `Drop` guarantees the datagram is dropped regardless of the
    /// profile's loss probability, but corruption for that same datagram
    /// is still a normal random draw unless a `Corrupt` rule also exists.
    pub fn send(&mut self, payload: &[u8]) {
        let index = self.next_datagram_index;
        self.next_datagram_index += 1;
        self.stats.sent += 1;
        let rules: Vec<ScriptedFault> = self.script.for_index(index).collect();

        let scripted_drop = rules.contains(&ScriptedFault::Drop);
        let dropped = scripted_drop || self.rng.chance(self.profile.loss);
        if dropped {
            self.stats.dropped += 1;
            return;
        }

        let base = self.emit_one(payload, &rules);
        self.enqueue(base);

        let scripted_dup = rules.contains(&ScriptedFault::Duplicate);
        let random_dup = !scripted_dup && self.rng.chance(self.profile.duplication);
        if scripted_dup || random_dup {
            self.stats.duplicated += 1;
            let dup = self.emit_one(payload, &rules);
            self.enqueue(dup);
        }
    }

    /// Builds one wire datagram from `payload`, applying delay/corrupt/
    /// truncate (scripted override, else random draw) and returning it
    /// ready to enqueue.
    fn emit_one(&mut self, payload: &[u8], rules: &[ScriptedFault]) -> InFlight {
        let mut bytes = payload.to_vec();

        let scripted_delay = rules.iter().find_map(|r| match r {
            ScriptedFault::Delay(d) => Some(*d),
            _ => None,
        });
        let extra_delay = scripted_delay.unwrap_or_else(|| {
            if self.profile.reorder_max_delay > 0 {
                self.rng.below(self.profile.reorder_max_delay + 1)
            } else {
                0
            }
        });
        if extra_delay > 0 {
            self.stats.reordered += 1;
        }

        let scripted_corrupt = rules.iter().find_map(|r| match r {
            ScriptedFault::Corrupt { byte_offset } => Some(*byte_offset),
            _ => None,
        });
        let should_corrupt = scripted_corrupt.is_some() || self.rng.chance(self.profile.corruption);
        if should_corrupt && !bytes.is_empty() {
            let offset = scripted_corrupt
                .unwrap_or_else(|| self.rng.below(bytes.len() as u64) as usize)
                % bytes.len();
            let bit = self.rng.below(8) as u8;
            bytes[offset] ^= 1 << bit;
            self.stats.corrupted += 1;
        }

        let scripted_trunc = rules.iter().find_map(|r| match r {
            ScriptedFault::Truncate { to_len } => Some(*to_len),
            _ => None,
        });
        let should_trunc = scripted_trunc.is_some() || self.rng.chance(self.profile.truncation);
        if should_trunc {
            let to_len = scripted_trunc
                .unwrap_or_else(|| {
                    if bytes.is_empty() {
                        0
                    } else {
                        self.rng.below(bytes.len() as u64) as usize
                    }
                })
                .min(bytes.len());
            if to_len < bytes.len() {
                bytes.truncate(to_len);
                self.stats.truncated += 1;
            }
        }

        InFlight {
            arrives: self.now.advance(self.profile.base_delay + extra_delay),
            seq: 0, // filled in by enqueue
            payload: bytes,
        }
    }

    fn enqueue(&mut self, mut datagram: InFlight) {
        let seq = self.payloads.len() as u64;
        datagram.seq = seq;
        let id = self.payloads.len();
        let arrives = datagram.arrives;
        self.payloads.push(datagram.payload);
        self.in_flight.push(Reverse((arrives, seq, id)));
    }

    /// Advances virtual time to `to` (a no-op if `to <= now()`) and
    /// returns every datagram whose arrival tick is now `<= to`, in
    /// arrival order (ties broken by send order). This is the *only* way
    /// time moves on this channel.
    pub fn advance(&mut self, to: Tick) -> Vec<Vec<u8>> {
        if to > self.now {
            self.now = to;
        }
        self.deliver_ready()
    }

    /// Every datagram deliverable at the current tick, without advancing
    /// time further.
    pub fn deliver_ready(&mut self) -> Vec<Vec<u8>> {
        let mut ready = Vec::new();
        while let Some(&Reverse((arrives, _, id))) = self.in_flight.peek() {
            if arrives > self.now {
                break;
            }
            self.in_flight.pop();
            ready.push(std::mem::take(&mut self.payloads[id]));
            self.stats.delivered += 1;
        }
        ready
    }

    /// The arrival tick of the next in-flight datagram, if any — lets a
    /// caller jump straight to the next interesting tick instead of
    /// single-stepping.
    pub fn next_arrival(&self) -> Option<Tick> {
        self.in_flight.peek().map(|Reverse((t, _, _))| *t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_channel_delivers_everything_unmodified_after_base_delay() {
        let mut ch = Channel::new(1, FaultProfile::CLEAN);
        ch.send(b"hello");
        ch.send(b"world");
        assert_eq!(ch.advance(Tick(0)), Vec::<Vec<u8>>::new());
        let delivered = ch.advance(Tick(1));
        assert_eq!(delivered, vec![b"hello".to_vec(), b"world".to_vec()]);
        assert_eq!(ch.stats().sent, 2);
        assert_eq!(ch.stats().delivered, 2);
        assert_eq!(ch.stats().dropped, 0);
    }

    #[test]
    fn total_loss_drops_everything() {
        let mut profile = FaultProfile::CLEAN;
        profile.loss = 1.0;
        let mut ch = Channel::new(1, profile);
        for _ in 0..20 {
            ch.send(b"x");
        }
        assert_eq!(ch.advance(Tick(100)), Vec::<Vec<u8>>::new());
        assert_eq!(ch.stats().dropped, 20);
        assert_eq!(ch.stats().delivered, 0);
    }

    #[test]
    fn total_duplication_delivers_everything_twice() {
        let mut profile = FaultProfile::CLEAN;
        profile.duplication = 1.0;
        let mut ch = Channel::new(1, profile);
        ch.send(b"x");
        let delivered = ch.advance(Tick(1));
        assert_eq!(delivered, vec![b"x".to_vec(), b"x".to_vec()]);
        assert_eq!(ch.stats().duplicated, 1);
    }

    #[test]
    fn total_corruption_always_changes_the_payload() {
        let mut profile = FaultProfile::CLEAN;
        profile.corruption = 1.0;
        let mut ch = Channel::new(3, profile);
        for _ in 0..50 {
            ch.send(b"deadbeef");
        }
        let delivered = ch.advance(Tick(1));
        assert_eq!(delivered.len(), 50);
        assert!(delivered.iter().all(|d| d.as_slice() != b"deadbeef"));
        assert_eq!(ch.stats().corrupted, 50);
    }

    #[test]
    fn total_truncation_always_shortens_a_nonempty_payload() {
        let mut profile = FaultProfile::CLEAN;
        profile.truncation = 1.0;
        let mut ch = Channel::new(5, profile);
        for _ in 0..50 {
            ch.send(b"deadbeef");
        }
        let delivered = ch.advance(Tick(1));
        assert!(delivered.iter().all(|d| d.len() < 8));
        assert_eq!(ch.stats().truncated, 50);
    }

    #[test]
    fn reordering_can_deliver_a_later_send_first() {
        let mut profile = FaultProfile::CLEAN;
        profile.reorder_max_delay = 20;
        // Try several seeds; at least one must show real reordering (this
        // is a property of the mechanism, not a guarantee for any single
        // seed, so we scan for a witness rather than asserting on one).
        let mut found = false;
        for seed in 0..50 {
            let mut ch = Channel::new(seed, profile);
            ch.send(b"first");
            ch.send(b"second");
            let delivered = ch.advance(Tick(200));
            if delivered == vec![b"second".to_vec(), b"first".to_vec()] {
                found = true;
                break;
            }
        }
        assert!(found, "expected at least one seed to reorder two sends");
    }

    #[test]
    fn deliver_ready_never_returns_a_datagram_before_its_tick() {
        let mut profile = FaultProfile::CLEAN;
        profile.base_delay = 10;
        let mut ch = Channel::new(2, profile);
        ch.send(b"x");
        assert!(ch.advance(Tick(9)).is_empty());
        assert_eq!(ch.advance(Tick(10)), vec![b"x".to_vec()]);
    }

    #[test]
    fn next_arrival_reports_the_soonest_pending_datagram() {
        let mut profile = FaultProfile::CLEAN;
        profile.base_delay = 5;
        let mut ch = Channel::new(1, profile);
        assert_eq!(ch.next_arrival(), None);
        ch.send(b"x");
        assert_eq!(ch.next_arrival(), Some(Tick(5)));
        ch.advance(Tick(5));
        assert_eq!(ch.next_arrival(), None);
    }

    #[test]
    fn a_scripted_drop_overrides_a_zero_loss_profile() {
        let script = Script::new().at(1, ScriptedFault::Drop);
        let mut ch = Channel::with_script(1, FaultProfile::CLEAN, script);
        ch.send(b"a"); // index 0, unaffected
        ch.send(b"b"); // index 1, scripted drop
        ch.send(b"c"); // index 2, unaffected
        let delivered = ch.advance(Tick(1));
        assert_eq!(delivered, vec![b"a".to_vec(), b"c".to_vec()]);
        assert_eq!(ch.stats().dropped, 1);
    }

    #[test]
    fn a_scripted_corrupt_targets_an_exact_byte() {
        let script = Script::new().at(0, ScriptedFault::Corrupt { byte_offset: 0 });
        let mut ch = Channel::with_script(9, FaultProfile::CLEAN, script);
        ch.send(&[0u8]);
        let delivered = ch.advance(Tick(1));
        assert_eq!(delivered.len(), 1);
        assert_ne!(delivered[0][0], 0);
    }

    #[test]
    fn a_scripted_truncate_gives_an_exact_length() {
        let script = Script::new().at(0, ScriptedFault::Truncate { to_len: 2 });
        let mut ch = Channel::with_script(9, FaultProfile::CLEAN, script);
        ch.send(b"deadbeef");
        let delivered = ch.advance(Tick(1));
        assert_eq!(delivered[0], b"de");
    }
}
