//! Transport's own small sub-header, carried inside `frame::Frame`'s
//! payload (frame's own fields carry the sequence number and the
//! SYN/FIN/ACK/RST control flags — see `docs/design/decisions/
//! ADR-003-reliable-transport.md` for why control flags don't consume
//! sequence-number space here, unlike real TCP).
//!
//! Layout (little-endian), prepended to any application data:
//! ```text
//! bytes 0..4:  cumulative ack (next contiguous byte expected)
//! byte  4:     sack range count (0..=MAX_SACK_RANGES)
//! then, per range: (start: u32, end: u32) — inclusive byte offsets,
//! sorted ascending, non-overlapping, each entirely beyond the cumulative
//! ack.
//! ```

pub const MAX_SACK_RANGES: usize = 4;
const FIXED_LEN: usize = 6; // ack(4) + fin_ack(1) + sack count(1)
const RANGE_LEN: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SackRange {
    pub start: u32,
    pub end: u32,
}

/// `fin_ack` is a dedicated bit (distinct from `frame::Frame`'s own
/// FIN/ACK flags) meaning "I specifically acknowledge receipt of your
/// FIN" — see `docs/design/decisions/ADR-003-reliable-transport.md` for
/// why FIN's reliability is tracked separately from data's, with its own
/// explicit acknowledgment, rather than consuming byte-sequence space
/// the way TCP's FIN does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SegmentHeader {
    pub ack: u32,
    pub fin_ack: bool,
    pub sack: Vec<SackRange>,
}

impl SegmentHeader {
    pub fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.ack.to_le_bytes());
        out.push(u8::from(self.fin_ack));
        let count = self.sack.len().min(MAX_SACK_RANGES) as u8;
        out.push(count);
        for r in self.sack.iter().take(MAX_SACK_RANGES) {
            out.extend_from_slice(&r.start.to_le_bytes());
            out.extend_from_slice(&r.end.to_le_bytes());
        }
    }

    /// Decodes a header from the front of `data`, returning it plus
    /// however many bytes of `data` remain (the application payload).
    /// `None` if `data` is too short to hold even the fixed part, or a
    /// declared range count that would run past the end.
    pub fn decode(data: &[u8]) -> Option<(SegmentHeader, &[u8])> {
        if data.len() < FIXED_LEN {
            return None;
        }
        let ack = u32::from_le_bytes(data[0..4].try_into().unwrap());
        let fin_ack = data[4] != 0;
        let count = data[5] as usize;
        let ranges_len = count * RANGE_LEN;
        if data.len() < FIXED_LEN + ranges_len {
            return None;
        }
        let mut sack = Vec::with_capacity(count);
        let mut offset = FIXED_LEN;
        for _ in 0..count {
            let start = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
            let end = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
            sack.push(SackRange { start, end });
            offset += RANGE_LEN;
        }
        Some((SegmentHeader { ack, fin_ack, sack }, &data[offset..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_with_no_sack_ranges() {
        let h = SegmentHeader {
            ack: 42,
            fin_ack: false,
            sack: vec![],
        };
        let mut buf = Vec::new();
        h.encode(&mut buf);
        let (decoded, rest) = SegmentHeader::decode(&buf).unwrap();
        assert_eq!(decoded, h);
        assert!(rest.is_empty());
    }

    #[test]
    fn round_trips_with_fin_ack_set() {
        let h = SegmentHeader {
            ack: 1,
            fin_ack: true,
            sack: vec![],
        };
        let mut buf = Vec::new();
        h.encode(&mut buf);
        assert_eq!(SegmentHeader::decode(&buf).unwrap().0, h);
    }

    #[test]
    fn round_trips_with_sack_ranges_and_trailing_payload() {
        let h = SegmentHeader {
            ack: 7,
            fin_ack: false,
            sack: vec![
                SackRange { start: 10, end: 19 },
                SackRange { start: 30, end: 39 },
            ],
        };
        let mut buf = Vec::new();
        h.encode(&mut buf);
        buf.extend_from_slice(b"payload");
        let (decoded, rest) = SegmentHeader::decode(&buf).unwrap();
        assert_eq!(decoded, h);
        assert_eq!(rest, b"payload");
    }

    #[test]
    fn decode_rejects_too_short_input() {
        assert_eq!(SegmentHeader::decode(&[1, 2, 3]), None);
    }

    #[test]
    fn decode_rejects_a_range_count_that_overruns_the_buffer() {
        let mut buf = vec![0u8; 5];
        buf.push(2); // claims 2 ranges = 16 more bytes, none present
        assert_eq!(SegmentHeader::decode(&buf), None);
    }

    #[test]
    fn encode_caps_at_max_sack_ranges() {
        let h = SegmentHeader {
            ack: 0,
            fin_ack: false,
            sack: (0..10).map(|i| SackRange { start: i, end: i }).collect(),
        };
        let mut buf = Vec::new();
        h.encode(&mut buf);
        assert_eq!(buf[5] as usize, MAX_SACK_RANGES);
        assert_eq!(buf.len(), FIXED_LEN + MAX_SACK_RANGES * RANGE_LEN);
    }
}
