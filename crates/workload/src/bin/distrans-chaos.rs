//! Runs the closing workload's chaos harness from the command line.
//!
//! ```text
//! distrans-chaos [--seed N] [--runs R] [--profile NAME] [--calls C] [--clients N]
//! ```
//!
//! Runs seeds `N, N+1, ..., N+R-1` against the named profile (default
//! `clean`; see `workload::profiles::all()` for the list). On the first
//! failure, prints it (including the exact command to replay it) and
//! exits 1.

use std::process::ExitCode;

use workload::chaos::{run_chaos, ChaosConfig};
use workload::profiles;

fn parse_u64(flag: &str, value: Option<String>) -> Result<u64, String> {
    let value = value.ok_or_else(|| format!("{flag} needs a value"))?;
    let parsed = match value.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => value.parse(),
    };
    parsed.map_err(|e| format!("{flag} {value}: {e}"))
}

fn main() -> ExitCode {
    let mut seed = 0u64;
    let mut runs = 1u64;
    let mut profile_name = "clean".to_string();
    let mut calls = 60usize;
    let mut clients = 3usize;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let result: Result<(), String> = match flag.as_str() {
            "--seed" => parse_u64(&flag, args.next()).map(|v| seed = v),
            "--runs" => parse_u64(&flag, args.next()).map(|v| runs = v),
            "--calls" => parse_u64(&flag, args.next()).map(|v| calls = v as usize),
            "--clients" => parse_u64(&flag, args.next()).map(|v| clients = v as usize),
            "--profile" => args
                .next()
                .ok_or_else(|| "--profile needs a value".to_string())
                .map(|v| profile_name = v),
            "--help" | "-h" => {
                println!("distrans-chaos [--seed N] [--runs R] [--profile NAME] [--calls C] [--clients N]");
                println!(
                    "profiles: {}",
                    profiles::all()
                        .iter()
                        .map(|(n, _)| *n)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                return ExitCode::SUCCESS;
            }
            other => Err(format!("unknown argument {other}")),
        };
        if let Err(e) = result {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    }

    let Some((_, profile)) = profiles::all()
        .into_iter()
        .find(|(n, _)| *n == profile_name)
    else {
        eprintln!("error: unknown profile {profile_name}");
        return ExitCode::from(2);
    };

    let mut total_executions = 0usize;
    let mut total_failed_calls = 0usize;
    for i in 0..runs {
        let config = ChaosConfig {
            seed: seed.wrapping_add(i),
            clients,
            calls,
            initial_profile: profile,
            max_ticks: 300_000,
            ..Default::default()
        };
        match run_chaos(config) {
            Ok(summary) => {
                total_executions += summary.executions;
                total_failed_calls += summary.calls_failed;
                if runs == 1 {
                    println!("seed {:#x} on {profile_name}: {:?}", summary.seed, summary);
                }
            }
            Err(failure) => {
                eprintln!("{failure}");
                return ExitCode::FAILURE;
            }
        }
    }
    if runs > 1 {
        println!(
            "{runs} seeds x {calls} calls x {clients} clients on {profile_name} clean (seeds {seed:#x}..{:#x}); {total_executions} handler executions, {total_failed_calls} legitimate call failures",
            seed.wrapping_add(runs - 1)
        );
    }
    ExitCode::SUCCESS
}
