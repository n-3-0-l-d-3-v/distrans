---
status: open
phase: 5
---

# 001 — The simulated hostile channel

The foundation everything else in this repo runs over: a datagram channel
that guarantees nothing, with virtual time and seeded, reproducible
fault injection, per `docs/design/WIRE.md`.

## Scope
- `Tick`: virtual time, advanced only by the simulator. No layer in this
  repo may read physical time (mirrors muaddib's `no_wall_clock` guard —
  this repo gets an analogous `no_wall_clock` test).
- `Channel`: an event-driven virtual-time queue of in-flight datagrams
  between two endpoints, each carrying an arrival tick.
- A `FaultProfile`: independent probabilities/parameters for loss,
  duplication, reordering (via randomized extra delay), corruption
  (bit-flip within the payload), and truncation.
- Seeded fault injection: a hand-rolled deterministic PRNG (no external
  `rand` dependency, so a recorded seed can't stop reproducing after a
  dependency bump — see muaddib's `workload::rng::SplitMix64` for the
  precedent).
- A scripted-fault mode: an explicit list of "at datagram N, do X" rules,
  for deterministic regression tests of one specific corner case,
  independent of the random profile.
- Per-channel fault statistics (datagrams sent/delivered/dropped/
  duplicated/reordered/corrupted/truncated) for observability.
- Property test: for an arbitrary seed and fault profile, the resulting
  sequence of channel events is reproduced exactly by a second run with
  the same seed and profile.
- Property test: each fault's observed rate over a large number of
  datagrams is statistically consistent with its configured probability.

Not started. No dependencies (first ticket of the phase).
