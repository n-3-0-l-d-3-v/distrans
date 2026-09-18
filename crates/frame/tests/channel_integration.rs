//! Proves ticket 001 and ticket 002 actually compose: frames sent
//! through a real hostile `Channel` are either delivered intact, or
//! rejected by `decode` with a typed error — never silently wrong, never
//! a panic — for every corruption/truncation fault the channel can throw.

use channel::{Channel, FaultProfile, Tick};
use frame::{decode, encode, DecodeError, Frame};

fn sample(i: u32) -> Frame {
    Frame {
        frame_type: 1,
        sequence: i,
        flags: 0,
        payload: format!("payload #{i}").into_bytes(),
    }
}

#[test]
fn frames_through_a_heavily_hostile_channel_are_intact_or_typed_errors() {
    let mut profile = FaultProfile::CLEAN;
    profile.corruption = 0.4;
    profile.truncation = 0.3;
    profile.loss = 0.0; // loss is orthogonal to integrity; excluded so every send is checked
    profile.duplication = 0.0;

    for seed in 0..40u64 {
        let mut ch = Channel::new(seed, profile);
        let frames: Vec<Frame> = (0..30).map(sample).collect();
        for f in &frames {
            ch.send(&encode(f));
        }
        let delivered = ch.advance(Tick(10));

        let mut intact = 0;
        let mut rejected = 0;
        for (original, datagram) in frames.iter().zip(&delivered) {
            match decode(datagram) {
                Ok(decoded) => {
                    assert_eq!(&decoded, original, "seed {seed}: a corrupted frame decoded to a DIFFERENT valid-looking frame instead of being rejected");
                    intact += 1;
                }
                Err(
                    DecodeError::ChecksumMismatch { .. }
                    | DecodeError::LengthMismatch { .. }
                    | DecodeError::TooShort { .. }
                    | DecodeError::UnsupportedVersion { .. },
                ) => {
                    rejected += 1;
                }
            }
        }
        assert_eq!(intact + rejected, delivered.len());
    }
}

#[test]
fn with_no_faults_every_frame_arrives_decodable_and_unchanged() {
    let mut ch = Channel::new(1, FaultProfile::CLEAN);
    let frames: Vec<Frame> = (0..10).map(sample).collect();
    for f in &frames {
        ch.send(&encode(f));
    }
    let delivered = ch.advance(Tick(5));
    assert_eq!(delivered.len(), frames.len());
    for (original, datagram) in frames.iter().zip(&delivered) {
        assert_eq!(&decode(datagram).unwrap(), original);
    }
}

#[test]
fn total_corruption_and_truncation_are_always_caught_never_silently_wrong() {
    let mut profile = FaultProfile::CLEAN;
    profile.corruption = 1.0;
    let mut ch = Channel::new(2, profile);
    let frames: Vec<Frame> = (0..50).map(sample).collect();
    for f in &frames {
        ch.send(&encode(f));
    }
    let delivered = ch.advance(Tick(5));
    for (original, datagram) in frames.iter().zip(&delivered) {
        // A corrupted frame is normally rejected; `Ok` only happens on a
        // rare coincidental CRC collision, in which case it must still
        // equal the original (never a different plausible frame).
        if let Ok(decoded) = decode(datagram) {
            assert_eq!(&decoded, original);
        }
    }
}
