---
status: open
phase: 5
---

# 006 — Integration workload, differential testing, and benchmarks

The Definition-of-Done items this repo hasn't earned even once tickets
001–005 close: a real observable workload, and measured comparisons.

## Scope
- A real RPC key-value service (`get`/`put`/`delete`) built on ticket
  005's RPC layer, driven by a multi-client workload through several
  named hostile-channel profiles (clean, lossy, high-latency/reordering,
  bursty-corruption, near-partition).
- Two independent reference implementations for comparison, run over the
  same hostile channel: a naive stop-and-wait sender, and a fixed-window
  go-back-N sender. Goodput (and, where meaningful, latency) measured
  against distrans's own selective-repeat + congestion-controlled
  transport across the same loss-rate sweep — numbers, reported honestly
  even if a simpler scheme wins in some regime.
- Seeded chaos testing: arbitrary combinations of fault profile, client
  concurrency and mid-stream profile changes, checked against the
  key-value service's own consistency model, replayable from a seed
  (same style as muaddib's `muaddib-chaos`).
- The closing ADR answers the phase's research question directly: which
  TCP mechanisms were *forced* by the hostile channel (a property
  demonstrably fails without them) versus *chosen* by convention (an
  alternative was measured and works), with the evidence for each claim
  named ticket-by-ticket.

Not started. Depends on tickets 001–005.
