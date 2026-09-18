//! Two textbook ARQ schemes, built **directly** on `frame`/`channel` —
//! deliberately *not* reusing anything from `transport` — so ticket 006's
//! comparison is of the scheme (selective-repeat + AIMD vs. these), not
//! of one shared implementation wearing different hats. Both use fixed
//! (non-adaptive) retransmission timeouts, unlike `transport`'s
//! Jacobson/Karels estimator — a second, stated simplification: these
//! exist to compare architectures, not to be equally sophisticated
//! implementations.
//!
//! Segments are numbered by index here (0, 1, 2, ...), not by byte
//! offset — the classic textbook framing for both schemes — using
//! `frame::Frame::sequence` directly. `flags & 1` marks an ACK frame
//! (its `sequence` is the next segment the receiver expects); a data
//! frame's payload is exactly one chunk.

use channel::{Channel, Tick};
use frame::{decode, encode, Frame};

const FLAG_ACK: u16 = 1;

#[derive(Debug, Clone, Copy)]
pub struct ArqConfig {
    pub segment_size: usize,
    pub rto: u64,
    pub max_retries: u32,
}

#[derive(Debug, Clone)]
pub struct RunResult {
    pub delivered: Vec<u8>,
    pub ticks_used: u64,
    pub completed: bool,
}

fn data_frame(seq: u32, payload: &[u8]) -> Vec<u8> {
    encode(&Frame {
        frame_type: 0,
        sequence: seq,
        flags: 0,
        payload: payload.to_vec(),
    })
}

fn ack_frame(next_expected: u32) -> Vec<u8> {
    encode(&Frame {
        frame_type: 0,
        sequence: next_expected,
        flags: FLAG_ACK,
        payload: Vec::new(),
    })
}

/// Stop-and-wait: exactly one segment in flight at a time.
pub fn run_stop_and_wait(
    data: &[u8],
    fwd: &mut Channel,
    back: &mut Channel,
    config: ArqConfig,
    max_ticks: u64,
) -> RunResult {
    let chunks: Vec<&[u8]> = data.chunks(config.segment_size.max(1)).collect();
    let mut next_seq = 0u32;
    let mut unacked: Option<(u32, Vec<u8>, Tick)> = None;
    let mut retries = 0u32;

    let mut expected = 0u32;
    let mut delivered = Vec::new();

    let mut tick = 0u64;
    loop {
        tick += 1;
        let now = Tick(tick);

        for datagram in fwd.advance(now) {
            if let Ok(f) = decode(&datagram) {
                if f.sequence == expected {
                    delivered.extend_from_slice(&f.payload);
                    expected += 1;
                }
                // Whether fresh or duplicate, tell the sender the current
                // cumulative point — stop-and-wait's receiver never buffers.
                back.send(&ack_frame(expected));
            }
        }
        for datagram in back.advance(now) {
            if let Ok(f) = decode(&datagram) {
                if f.flags & FLAG_ACK != 0 {
                    if let Some((seq, _, _)) = &unacked {
                        if f.sequence == seq + 1 {
                            unacked = None;
                            retries = 0;
                        }
                    }
                }
            }
        }

        if unacked.is_none() && (next_seq as usize) < chunks.len() {
            let chunk = chunks[next_seq as usize].to_vec();
            fwd.send(&data_frame(next_seq, &chunk));
            unacked = Some((next_seq, chunk, now));
            next_seq += 1;
        } else if let Some((seq, chunk, sent_at)) = &unacked {
            if now - *sent_at >= config.rto {
                retries += 1;
                if retries > config.max_retries {
                    return RunResult {
                        delivered,
                        ticks_used: tick,
                        completed: false,
                    };
                }
                fwd.send(&data_frame(*seq, chunk));
                unacked = Some((*seq, chunk.clone(), now));
            }
        }

        let done = unacked.is_none() && (next_seq as usize) == chunks.len();
        if done || tick >= max_ticks {
            return RunResult {
                delivered,
                ticks_used: tick,
                completed: done,
            };
        }
    }
}

/// Go-back-N: up to `window` segments in flight; a timeout on the oldest
/// unacked segment retransmits the *entire* unacked window (no selective
/// repeat); the receiver only accepts segments strictly in order,
/// discarding (not buffering) anything else.
pub fn run_go_back_n(
    data: &[u8],
    fwd: &mut Channel,
    back: &mut Channel,
    config: ArqConfig,
    window: usize,
    max_ticks: u64,
) -> RunResult {
    let chunks: Vec<Vec<u8>> = data
        .chunks(config.segment_size.max(1))
        .map(|c| c.to_vec())
        .collect();
    let mut base = 0u32;
    let mut next_seq = 0u32;
    let mut oldest_sent_at: Option<Tick> = None;
    let mut retries = 0u32;

    let mut expected = 0u32;
    let mut delivered = Vec::new();

    let mut tick = 0u64;
    loop {
        tick += 1;
        let now = Tick(tick);

        for datagram in fwd.advance(now) {
            if let Ok(f) = decode(&datagram) {
                if f.sequence == expected {
                    delivered.extend_from_slice(&f.payload);
                    expected += 1;
                }
                back.send(&ack_frame(expected));
            }
        }
        for datagram in back.advance(now) {
            if let Ok(f) = decode(&datagram) {
                if f.flags & FLAG_ACK != 0 && f.sequence > base {
                    base = f.sequence;
                    retries = 0;
                    oldest_sent_at = if base < next_seq { Some(now) } else { None };
                }
            }
        }

        while next_seq < base + window as u32 && (next_seq as usize) < chunks.len() {
            fwd.send(&data_frame(next_seq, &chunks[next_seq as usize]));
            if oldest_sent_at.is_none() {
                oldest_sent_at = Some(now);
            }
            next_seq += 1;
        }

        if let Some(sent_at) = oldest_sent_at {
            if now - sent_at >= config.rto {
                retries += 1;
                if retries > config.max_retries {
                    return RunResult {
                        delivered,
                        ticks_used: tick,
                        completed: false,
                    };
                }
                for seq in base..next_seq {
                    fwd.send(&data_frame(seq, &chunks[seq as usize]));
                }
                oldest_sent_at = Some(now);
            }
        }

        let done = base as usize == chunks.len();
        if done || tick >= max_ticks {
            return RunResult {
                delivered,
                ticks_used: tick,
                completed: done,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use channel::FaultProfile;

    fn cfg() -> ArqConfig {
        ArqConfig {
            segment_size: 32,
            rto: 10,
            max_retries: 30,
        }
    }

    #[test]
    fn stop_and_wait_delivers_correctly_on_a_clean_channel() {
        let mut fwd = Channel::new(1, FaultProfile::CLEAN);
        let mut back = Channel::new(2, FaultProfile::CLEAN);
        let data: Vec<u8> = (0..500u32).map(|i| i as u8).collect();
        let result = run_stop_and_wait(&data, &mut fwd, &mut back, cfg(), 5000);
        assert!(result.completed);
        assert_eq!(result.delivered, data);
    }

    #[test]
    fn go_back_n_delivers_correctly_on_a_clean_channel() {
        let mut fwd = Channel::new(1, FaultProfile::CLEAN);
        let mut back = Channel::new(2, FaultProfile::CLEAN);
        let data: Vec<u8> = (0..2000u32).map(|i| i as u8).collect();
        let result = run_go_back_n(&data, &mut fwd, &mut back, cfg(), 8, 5000);
        assert!(result.completed);
        assert_eq!(result.delivered, data);
    }

    #[test]
    fn both_schemes_deliver_correctly_under_moderate_loss() {
        let mut profile = FaultProfile::CLEAN;
        profile.loss = 0.1;
        profile.corruption = 0.05;
        for seed in 0..15u64 {
            let mut fwd = Channel::new(seed, profile);
            let mut back = Channel::new(seed + 500, profile);
            let data: Vec<u8> = (0..600u32).map(|i| (i % 200) as u8).collect();
            let r1 = run_stop_and_wait(&data, &mut fwd, &mut back, cfg(), 20_000);
            assert!(r1.completed, "stop-and-wait seed {seed}");
            assert_eq!(r1.delivered, data, "stop-and-wait seed {seed}");

            let mut fwd = Channel::new(seed, profile);
            let mut back = Channel::new(seed + 500, profile);
            let r2 = run_go_back_n(&data, &mut fwd, &mut back, cfg(), 8, 20_000);
            assert!(r2.completed, "go-back-n seed {seed}");
            assert_eq!(r2.delivered, data, "go-back-n seed {seed}");
        }
    }

    #[test]
    fn go_back_n_is_never_slower_than_stop_and_wait_on_a_clean_channel() {
        // Not a hard invariant of every possible parameterization, but
        // true for any reasonable window > 1 on a lossless channel, and
        // a useful sanity check that the window is actually doing
        // something (otherwise this comparison in ADR-006 would be
        // measuring nothing).
        let data: Vec<u8> = (0..4000u32).map(|i| i as u8).collect();
        let mut fwd = Channel::new(9, FaultProfile::CLEAN);
        let mut back = Channel::new(10, FaultProfile::CLEAN);
        let saw = run_stop_and_wait(&data, &mut fwd, &mut back, cfg(), 50_000);

        let mut fwd = Channel::new(9, FaultProfile::CLEAN);
        let mut back = Channel::new(10, FaultProfile::CLEAN);
        let gbn = run_go_back_n(&data, &mut fwd, &mut back, cfg(), 8, 50_000);

        assert!(gbn.ticks_used < saw.ticks_used);
    }
}
