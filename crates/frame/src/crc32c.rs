//! CRC-32C (Castagnoli polynomial, 0x1EDC6F41) — the same polynomial
//! iSCSI/SCTP use, chosen for its documented error-detection strength:
//! it's guaranteed to detect all single-bit and double-bit errors, all
//! errors of odd weight, and any burst error up to 32 bits, for messages
//! up to a few KB (comfortably covering this repo's frame sizes; see
//! Koopman's exhaustive analysis, "32-Bit Cyclic Redundancy Codes for
//! Internet Applications", 2002). Hand-rolled bit-at-a-time
//! implementation — not the fastest, but this repo isn't chasing CRC
//! throughput, and a from-scratch implementation is independent of any
//! external crate's own bugs, which matters since `frame::decode`'s
//! whole job is to be trustworthy.

const POLY: u32 = 0x82F6_3B78; // reversed (bit-reflected) 0x1EDC6F41

pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_test_vector() {
        // Standard CRC-32C check value for the ASCII string "123456789".
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn empty_input_has_a_fixed_value() {
        assert_eq!(crc32c(b""), 0);
    }

    #[test]
    fn different_inputs_usually_differ() {
        assert_ne!(crc32c(b"hello"), crc32c(b"world"));
    }

    #[test]
    fn every_single_bit_flip_changes_the_crc() {
        let base = b"the quick brown fox".to_vec();
        let base_crc = crc32c(&base);
        for byte_idx in 0..base.len() {
            for bit in 0..8u8 {
                let mut mutated = base.clone();
                mutated[byte_idx] ^= 1 << bit;
                assert_ne!(
                    crc32c(&mutated),
                    base_crc,
                    "single bit flip at byte {byte_idx} bit {bit} went undetected"
                );
            }
        }
    }
}
