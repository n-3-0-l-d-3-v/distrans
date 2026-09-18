//! Framing and integrity: turn a raw byte payload into a self-describing,
//! corruption-detecting wire unit, per `docs/design/WIRE.md` ticket 002.
//!
//! Wire format (all multi-byte integers little-endian):
//! ```text
//! byte 0:      version (currently only VERSION is accepted)
//! byte 1:      frame type (an opaque u8; this crate doesn't interpret it —
//!              transport/rpc give it meaning)
//! bytes 2..6:  sequence number (u32)
//! bytes 6..8:  flags (u16, bit-flag meaning left to the caller)
//! bytes 8..12: payload length (u32)
//! bytes 12..12+len: payload
//! last 4 bytes: CRC-32C over every preceding byte (header + payload)
//! ```
//! `decode` is the trust boundary for this whole repo: it must never
//! panic, and it must never return an `Ok` frame that wasn't actually
//! valid — every one of a truncated header, truncated payload, a length
//! field that doesn't match what's actually present, an unrecognized
//! version, and a bad checksum gets its own error variant.

mod crc32c;

use crc32c::crc32c;

pub const VERSION: u8 = 1;
const HEADER_LEN: usize = 12;
const CRC_LEN: usize = 4;
const MIN_FRAME_LEN: usize = HEADER_LEN + CRC_LEN;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub frame_type: u8,
    pub sequence: u32,
    pub flags: u16,
    pub payload: Vec<u8>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error(
        "datagram is {len} bytes, shorter than the {MIN_FRAME_LEN}-byte minimum (header + CRC)"
    )]
    TooShort { len: usize },
    #[error("unsupported frame version {found} (this build only decodes version {VERSION})")]
    UnsupportedVersion { found: u8 },
    #[error("header declares a payload of {declared} bytes, but only {available} bytes remain before the CRC trailer")]
    LengthMismatch { declared: u32, available: usize },
    #[error("CRC-32C mismatch: computed {computed:#010x}, frame claims {claimed:#010x}")]
    ChecksumMismatch { computed: u32, claimed: u32 },
}

/// Encodes `frame` to its wire representation. Always succeeds — there is
/// no payload-length limit within `u32`, and every field is already a
/// fixed-width integer, so there's nothing for this direction to reject.
pub fn encode(frame: &Frame) -> Vec<u8> {
    let mut out = Vec::with_capacity(MIN_FRAME_LEN + frame.payload.len());
    out.push(VERSION);
    out.push(frame.frame_type);
    out.extend_from_slice(&frame.sequence.to_le_bytes());
    out.extend_from_slice(&frame.flags.to_le_bytes());
    out.extend_from_slice(&(frame.payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&frame.payload);
    let crc = crc32c(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

/// Decodes a raw datagram back into a `Frame`. Checks are ordered so the
/// cheapest, most-certain-to-fire checks (length) run before the CRC scan
/// (which touches every byte): a datagram from the hostile channel that
/// is merely too short never pays for a full checksum pass over garbage.
pub fn decode(datagram: &[u8]) -> Result<Frame, DecodeError> {
    if datagram.len() < MIN_FRAME_LEN {
        return Err(DecodeError::TooShort {
            len: datagram.len(),
        });
    }
    let version = datagram[0];
    if version != VERSION {
        return Err(DecodeError::UnsupportedVersion { found: version });
    }
    let frame_type = datagram[1];
    let sequence = u32::from_le_bytes(datagram[2..6].try_into().unwrap());
    let flags = u16::from_le_bytes(datagram[6..8].try_into().unwrap());
    let declared_len = u32::from_le_bytes(datagram[8..12].try_into().unwrap());

    let available = datagram.len() - HEADER_LEN - CRC_LEN;
    if declared_len as usize != available {
        return Err(DecodeError::LengthMismatch {
            declared: declared_len,
            available,
        });
    }

    let payload_end = HEADER_LEN + available;
    let claimed_crc = u32::from_le_bytes(
        datagram[payload_end..payload_end + CRC_LEN]
            .try_into()
            .unwrap(),
    );
    let computed_crc = crc32c(&datagram[..payload_end]);
    if computed_crc != claimed_crc {
        return Err(DecodeError::ChecksumMismatch {
            computed: computed_crc,
            claimed: claimed_crc,
        });
    }

    Ok(Frame {
        frame_type,
        sequence,
        flags,
        payload: datagram[HEADER_LEN..payload_end].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Frame {
        Frame {
            frame_type: 7,
            sequence: 42,
            flags: 0b1010,
            payload: b"hello, hostile world".to_vec(),
        }
    }

    #[test]
    fn round_trips() {
        let f = sample();
        assert_eq!(decode(&encode(&f)), Ok(f));
    }

    #[test]
    fn round_trips_with_empty_payload() {
        let f = Frame {
            frame_type: 0,
            sequence: 0,
            flags: 0,
            payload: vec![],
        };
        assert_eq!(decode(&encode(&f)), Ok(f));
    }

    #[test]
    fn round_trips_at_max_u16_flags_and_type() {
        let f = Frame {
            frame_type: 255,
            sequence: u32::MAX,
            flags: u16::MAX,
            payload: vec![9; 300],
        };
        assert_eq!(decode(&encode(&f)), Ok(f));
    }

    #[test]
    fn empty_datagram_is_too_short_not_a_panic() {
        assert_eq!(decode(&[]), Err(DecodeError::TooShort { len: 0 }));
    }

    #[test]
    fn a_datagram_one_byte_short_of_minimum_is_too_short() {
        let datagram = vec![0u8; MIN_FRAME_LEN - 1];
        assert_eq!(
            decode(&datagram),
            Err(DecodeError::TooShort {
                len: MIN_FRAME_LEN - 1
            })
        );
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let mut d = encode(&sample());
        d[0] = 99;
        assert_eq!(
            decode(&d),
            Err(DecodeError::UnsupportedVersion { found: 99 })
        );
    }

    #[test]
    fn a_truncated_payload_is_a_length_mismatch_not_a_wrong_frame() {
        let mut d = encode(&sample());
        d.truncate(d.len() - 3); // still >= MIN_FRAME_LEN, mismatched length
        assert!(matches!(
            decode(&d),
            Err(DecodeError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn a_bit_flip_in_the_payload_is_caught_by_the_checksum() {
        let mut d = encode(&sample());
        let payload_byte = HEADER_LEN;
        d[payload_byte] ^= 1;
        assert!(matches!(
            decode(&d),
            Err(DecodeError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn a_bit_flip_in_the_header_is_caught_by_the_checksum() {
        let mut d = encode(&sample());
        d[2] ^= 1; // inside the sequence number field
        assert!(matches!(
            decode(&d),
            Err(DecodeError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn a_corrupted_length_field_is_caught_before_the_checksum_even_runs() {
        // A length field corrupted to claim more payload than is present:
        // must be LengthMismatch, since there aren't enough bytes to even
        // attempt a checksum over the claimed frame.
        let mut d = encode(&sample());
        d[8..12].copy_from_slice(&(sample().payload.len() as u32 + 1000).to_le_bytes());
        assert!(matches!(
            decode(&d),
            Err(DecodeError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn decode_never_panics_on_arbitrary_short_garbage() {
        for len in 0..40 {
            let garbage = vec![0xAAu8; len];
            let _ = decode(&garbage); // must not panic
        }
    }
}
