---
status: done
phase: 5
---

# 001 — The simulated hostile channel

The foundation everything else in this repo runs over: a datagram channel
that guarantees nothing, with virtual time and seeded, reproducible
fault injection, per `docs/design/WIRE.md`.

## Scope
- [x] `Tick`: virtual time, advanced only by the simulator.
- [x] `Channel`: an event-driven virtual-time queue of in-flight
      datagrams, each carrying an arrival tick.
- [x] `FaultProfile`: independent loss/duplication/reorder/corruption/
      truncation probabilities plus a base transit delay.
- [x] Seeded fault injection via a hand-rolled `SplitMix64` (no `rand`
      dependency, for cross-version-stable replay).
- [x] `Script`/`ScriptedFault`: explicit per-datagram-index overrides,
      additive with the random profile.
- [x] Per-channel `FaultStats` (sent/delivered/dropped/duplicated/
      reordered/corrupted/truncated).
- [x] Property test: a `(seed, profile)` run replays byte-for-byte and
      stats-for-stats identically —
      `same_seed_and_profile_reproduce_exactly` (256 cases).
- [x] Property test (statistical): each fault's observed rate matches its
      configured probability within tolerance —
      `observed_fault_rates_match_configured_probabilities` (5
      probabilities × 3 seeds × 4 fault kinds, N=20,000 each).
- [x] `no_wall_clock` guard test, mirroring muaddib's.

21 unit tests, 3 property tests, 1 statistical test, 1 guard test — all
mutation-checked (disabling loss injection is caught by two different
tests). See `docs/design/decisions/ADR-001-hostile-channel.md`.
