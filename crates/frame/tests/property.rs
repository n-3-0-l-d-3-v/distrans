//! Property tests for ticket 002's guarantees.

use frame::{decode, encode, DecodeError, Frame};
use proptest::prelude::*;

fn arb_frame() -> impl Strategy<Value = Frame> {
    (
        any::<u8>(),
        any::<u32>(),
        any::<u16>(),
        prop::collection::vec(any::<u8>(), 0..512),
    )
        .prop_map(|(frame_type, sequence, flags, payload)| Frame {
            frame_type,
            sequence,
            flags,
            payload,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// `decode(encode(f)) == Ok(f)` for arbitrary frames.
    #[test]
    fn round_trips_for_arbitrary_frames(f in arb_frame()) {
        prop_assert_eq!(decode(&encode(&f)), Ok(f));
    }

    /// Flipping any single bit anywhere in an encoded frame is always
    /// caught — either as a checksum mismatch, or (if the flip happened to
    /// land in the length field in a way that also changes the datagram's
    /// apparent structure) as a length mismatch. It must never silently
    /// decode to a *different* valid-looking frame.
    #[test]
    fn a_single_bit_flip_anywhere_is_always_caught(
        f in arb_frame(),
        byte_index in any::<usize>(),
        bit in 0u8..8,
    ) {
        let mut d = encode(&f);
        let idx = byte_index % d.len();
        d[idx] ^= 1 << bit;
        match decode(&d) {
            Ok(decoded) => prop_assert_eq!(decoded, f, "bit flip at byte {} bit {} silently produced a different valid frame", idx, bit),
            Err(DecodeError::ChecksumMismatch { .. } | DecodeError::LengthMismatch { .. } | DecodeError::UnsupportedVersion { .. }) => {}
            Err(other) => prop_assert!(false, "unexpected error variant: {other}"),
        }
    }

    /// Flipping an odd number of bits (1 or 3) anywhere never silently
    /// decodes as a different valid frame — CRC-32C detects all odd-weight
    /// errors.
    #[test]
    fn odd_weight_bit_flips_are_always_caught(
        f in arb_frame(),
        positions in prop::collection::vec((any::<usize>(), 0u8..8), 1..4),
    ) {
        prop_assume!(positions.len() % 2 == 1);
        let mut d = encode(&f);
        let len = d.len();
        let mut flipped = std::collections::HashSet::new();
        for (byte_index, bit) in &positions {
            let idx = byte_index % len;
            // Skip if this exact (byte, bit) was already flipped (would
            // cancel out and reduce the effective weight below odd).
            if !flipped.insert((idx, *bit)) {
                continue;
            }
            d[idx] ^= 1 << bit;
        }
        prop_assume!(!flipped.is_empty());
        if let Ok(decoded) = decode(&d) {
            prop_assert_eq!(decoded, f);
        }
    }

    /// Decoding an arbitrary truncated prefix of an encoded frame never
    /// panics, and never returns `Ok` with a value different from what a
    /// correct decode would give (a truncated prefix can only ever
    /// legitimately decode as `Ok` if, by sheer coincidence, it happens to
    /// re-form a different complete, checksum-valid frame — which the
    /// assertion below allows for by not requiring `Err`, only requiring
    /// no silent wrong-but-plausible decode of the *original* frame's
    /// truncated bytes).
    #[test]
    fn decoding_a_truncated_prefix_never_panics(
        f in arb_frame(),
        cut_at in any::<usize>(),
    ) {
        let d = encode(&f);
        let cut = cut_at % (d.len() + 1);
        let prefix = &d[..cut];
        let result = std::panic::catch_unwind(|| decode(prefix));
        prop_assert!(result.is_ok(), "decode panicked on a truncated prefix of length {cut}");
        if cut < d.len() {
            // A strictly truncated prefix is never a byte-for-byte encode
            // of any complete frame equal to f (encode(f) has length
            // exactly d.len()), so it must not equal Ok(f).
            if let Ok(Ok(decoded)) = result {
                prop_assert_ne!(decoded, f);
            }
        }
    }

    /// Decoding arbitrary random bytes of arbitrary length never panics.
    #[test]
    fn decoding_arbitrary_garbage_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..600)) {
        let result = std::panic::catch_unwind(|| decode(&bytes));
        prop_assert!(result.is_ok(), "decode panicked on arbitrary garbage");
    }
}
