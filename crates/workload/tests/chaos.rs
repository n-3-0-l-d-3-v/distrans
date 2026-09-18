//! The closing workload's chaos harness, run as part of the normal test
//! suite. See ADR-006 for the mutation-testing record (a broken `Delete`
//! result is caught immediately across every named profile) and larger
//! seed sweeps run via the `distrans-chaos` binary.

use workload::chaos::{run_chaos, ChaosConfig};
use workload::profiles;

#[test]
fn many_seeds_run_clean_on_every_named_profile() {
    for (name, profile) in profiles::all() {
        for seed in 0..12u64 {
            let cfg = ChaosConfig {
                seed,
                initial_profile: profile,
                calls: 30,
                max_ticks: 100_000,
                ..Default::default()
            };
            if let Err(failure) = run_chaos(cfg) {
                panic!("{name}: {failure}");
            }
        }
    }
}

#[test]
fn a_run_is_a_pure_function_of_its_seed() {
    let cfg = |seed| ChaosConfig {
        seed,
        calls: 25,
        ..Default::default()
    };
    let a = run_chaos(cfg(42)).unwrap();
    let b = run_chaos(cfg(42)).unwrap();
    assert_eq!(a, b, "same seed must reproduce identically");
    assert_ne!(a.ticks_used, 0);
}

#[test]
fn mid_stream_profile_switches_are_handled_correctly() {
    for seed in 0..15u64 {
        let cfg = ChaosConfig {
            seed,
            initial_profile: profiles::clean(),
            profile_switches: vec![
                (40, profiles::lossy()),
                (120, profiles::high_latency_reordering()),
                (220, profiles::bursty_corruption()),
            ],
            calls: 45,
            max_ticks: 150_000,
            ..Default::default()
        };
        if let Err(failure) = run_chaos(cfg) {
            panic!("seed {seed}: {failure}");
        }
    }
}

#[test]
fn near_partition_produces_legitimate_call_failures_not_invariant_violations() {
    let mut any_failed = false;
    for seed in 0..20u64 {
        let cfg = ChaosConfig {
            seed,
            initial_profile: profiles::near_partition(),
            calls: 20,
            max_ticks: 150_000,
            ..Default::default()
        };
        let summary = run_chaos(cfg).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        any_failed |= summary.calls_failed > 0;
    }
    assert!(
        any_failed,
        "near_partition (60% loss) never produced a single call failure across 20 seeds — \
         either the profile isn't as hostile as intended, or failures are being swallowed"
    );
}

#[test]
fn more_clients_and_a_larger_key_space_still_run_clean() {
    for seed in 0..8u64 {
        let cfg = ChaosConfig {
            seed,
            clients: 6,
            key_space: 16,
            calls: 80,
            initial_profile: profiles::lossy(),
            max_ticks: 200_000,
            ..Default::default()
        };
        if let Err(failure) = run_chaos(cfg) {
            panic!("seed {seed}: {failure}");
        }
    }
}
