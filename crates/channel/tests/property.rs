//! Property tests for ticket 001's two guarantees: a `(seed, profile)`
//! run is exactly reproducible, and each fault's observed rate is
//! statistically consistent with its configured probability.

use channel::{Channel, FaultProfile, Tick};
use proptest::prelude::*;

fn arb_profile() -> impl Strategy<Value = FaultProfile> {
    (
        0.0f64..1.0,
        0.0f64..1.0,
        0u64..8,
        0.0f64..1.0,
        0.0f64..1.0,
        0u64..5,
    )
        .prop_map(
            |(loss, duplication, reorder_max_delay, corruption, truncation, base_delay)| {
                FaultProfile {
                    loss,
                    duplication,
                    reorder_max_delay,
                    corruption,
                    truncation,
                    base_delay,
                }
            },
        )
}

fn run(
    seed: u64,
    profile: FaultProfile,
    payloads: &[Vec<u8>],
) -> (Vec<Vec<u8>>, channel::FaultStats) {
    let mut ch = Channel::new(seed, profile);
    for p in payloads {
        ch.send(p);
    }
    let delivered = ch.advance(Tick(1000));
    (delivered, ch.stats())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The same seed and profile, replayed against the same sends,
    /// produce byte-for-byte identical delivered output and identical
    /// stats — the reproducibility guarantee the whole repo's replay
    /// story depends on.
    #[test]
    fn same_seed_and_profile_reproduce_exactly(
        seed in any::<u64>(),
        profile in arb_profile(),
        payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..16), 1..20),
    ) {
        let a = run(seed, profile, &payloads);
        let b = run(seed, profile, &payloads);
        prop_assert_eq!(a, b);
    }

}

/// The RNG is actually consumed (not e.g. a stub returning a constant):
/// across many seeds at a mid-band probability and a decent-sized batch
/// of sends, the delivered outcomes are not all identical. A statistical
/// check like `observed_fault_rates_match_configured_probabilities`
/// below, not a per-input proptest property (two arbitrary seeds can
/// coincidentally agree on a single small input, which made this flaky
/// as a shrinkable property; at N=500 sends coincidental agreement across
/// every one of 16 seeds is not a realistic concern).
#[test]
fn different_seeds_produce_different_outcomes_at_a_mid_band_probability() {
    let mut profile = FaultProfile::CLEAN;
    profile.loss = 0.5;
    let payloads: Vec<Vec<u8>> = (0..500u32).map(|i| i.to_le_bytes().to_vec()).collect();

    let (baseline, _) = run(1000, profile, &payloads);
    let mut distinct = std::collections::HashSet::new();
    distinct.insert(baseline);
    for seed in 0..16u64 {
        distinct.insert(run(seed, profile, &payloads).0);
    }
    assert!(
        distinct.len() > 1,
        "16 seeds all agreed — RNG isn't being consumed"
    );
}

/// Statistical rate check: run enough datagrams through each fault in
/// isolation that the observed rate must land within a wide tolerance of
/// the configured probability. Not a proptest (a fixed large N gives a
/// stable statistical test; shrinking a flaky statistical assertion
/// isn't meaningful), but exercised across several probabilities and
/// seeds directly.
#[test]
fn observed_fault_rates_match_configured_probabilities() {
    const N: u64 = 20_000;
    const TOLERANCE: f64 = 0.03; // absolute, generous for N=20,000

    for &p in &[0.01, 0.1, 0.3, 0.5, 0.9] {
        for seed in [1u64, 2, 3] {
            let mut loss_profile = FaultProfile::CLEAN;
            loss_profile.loss = p;
            let mut ch = Channel::new(seed, loss_profile);
            for _ in 0..N {
                ch.send(b"x");
            }
            ch.advance(Tick(10));
            let rate = ch.stats().dropped as f64 / N as f64;
            assert!(
                (rate - p).abs() < TOLERANCE,
                "loss p={p} seed={seed}: observed {rate}, expected ~{p}"
            );

            let mut dup_profile = FaultProfile::CLEAN;
            dup_profile.duplication = p;
            let mut ch = Channel::new(seed, dup_profile);
            for _ in 0..N {
                ch.send(b"x");
            }
            ch.advance(Tick(10));
            let rate = ch.stats().duplicated as f64 / N as f64;
            assert!(
                (rate - p).abs() < TOLERANCE,
                "duplication p={p} seed={seed}: observed {rate}, expected ~{p}"
            );

            let mut corrupt_profile = FaultProfile::CLEAN;
            corrupt_profile.corruption = p;
            let mut ch = Channel::new(seed, corrupt_profile);
            for _ in 0..N {
                ch.send(b"deadbeef");
            }
            ch.advance(Tick(10));
            let rate = ch.stats().corrupted as f64 / N as f64;
            assert!(
                (rate - p).abs() < TOLERANCE,
                "corruption p={p} seed={seed}: observed {rate}, expected ~{p}"
            );

            let mut trunc_profile = FaultProfile::CLEAN;
            trunc_profile.truncation = p;
            let mut ch = Channel::new(seed, trunc_profile);
            for _ in 0..N {
                ch.send(b"deadbeef");
            }
            ch.advance(Tick(10));
            let rate = ch.stats().truncated as f64 / N as f64;
            assert!(
                (rate - p).abs() < TOLERANCE,
                "truncation p={p} seed={seed}: observed {rate}, expected ~{p}"
            );
        }
    }
}
