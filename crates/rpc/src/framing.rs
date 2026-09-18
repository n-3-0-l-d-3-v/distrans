//! `transport::Connection` delivers a reliable, ordered **byte stream**,
//! not discrete messages — `recv()` can return a partial message, several
//! coalesced messages, or anything in between, since bytes are segmented
//! into transport frames independently of any message boundary the
//! application cares about. RPC needs to see whole requests and
//! responses, so this module reassembles the stream into
//! length-prefixed frames (a 4-byte little-endian length, then that many
//! payload bytes) before anything else in this crate looks at it.

use std::collections::VecDeque;

const LEN_PREFIX: usize = 4;

/// Encodes one length-prefixed frame.
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(LEN_PREFIX + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Feeds a raw byte stream (as it arrives from `Connection::recv()`,
/// however it happens to be chunked) and yields complete frame payloads
/// as they become available.
#[derive(Debug, Default)]
pub struct FrameReader {
    buf: VecDeque<u8>,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend(bytes.iter().copied());
    }

    /// Every complete frame currently available, in order. Any trailing
    /// partial frame is left buffered for a future call.
    pub fn drain_frames(&mut self) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        loop {
            if self.buf.len() < LEN_PREFIX {
                break;
            }
            let len_bytes: Vec<u8> = self.buf.iter().take(LEN_PREFIX).copied().collect();
            let len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
            if self.buf.len() < LEN_PREFIX + len {
                break; // whole frame not here yet
            }
            self.buf.drain(..LEN_PREFIX);
            let payload: Vec<u8> = self.buf.drain(..len).collect();
            frames.push(payload);
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_frame_fed_whole_is_read_back() {
        let mut r = FrameReader::new();
        r.feed(&frame(b"hello"));
        assert_eq!(r.drain_frames(), vec![b"hello".to_vec()]);
    }

    #[test]
    fn a_frame_fed_one_byte_at_a_time_is_still_read_correctly() {
        let mut r = FrameReader::new();
        let encoded = frame(b"world");
        for b in &encoded {
            r.feed(&[*b]); // never panics mid-frame
        }
        assert_eq!(r.drain_frames(), vec![b"world".to_vec()]);
    }

    #[test]
    fn multiple_frames_concatenated_in_one_feed_are_all_read() {
        let mut r = FrameReader::new();
        let mut bytes = frame(b"a");
        bytes.extend(frame(b"bb"));
        bytes.extend(frame(b""));
        r.feed(&bytes);
        assert_eq!(
            r.drain_frames(),
            vec![b"a".to_vec(), b"bb".to_vec(), b"".to_vec()]
        );
    }

    #[test]
    fn a_trailing_partial_frame_is_held_until_the_rest_arrives() {
        let mut r = FrameReader::new();
        let encoded = frame(b"complete");
        r.feed(&encoded[..encoded.len() - 2]);
        assert!(r.drain_frames().is_empty());
        r.feed(&encoded[encoded.len() - 2..]);
        assert_eq!(r.drain_frames(), vec![b"complete".to_vec()]);
    }

    #[test]
    fn an_empty_payload_frame_round_trips() {
        let mut r = FrameReader::new();
        r.feed(&frame(&[]));
        assert_eq!(r.drain_frames(), vec![Vec::<u8>::new()]);
    }
}
