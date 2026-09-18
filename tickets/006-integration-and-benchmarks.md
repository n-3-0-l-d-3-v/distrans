---
status: done
phase: 5
---

# 006 — Integration workload, differential testing, and benchmarks

The Definition-of-Done items this repo hasn't earned even once tickets
001–005 close: a real observable workload, and measured comparisons.

## Scope
- [x] A real RPC key-value service (`get`/`put`/`delete`) built on
      ticket 005's RPC layer, driven by a multi-client workload through
      five named hostile-channel profiles (`clean`, `lossy`,
      `high_latency_reordering`, `bursty_corruption`, `near_partition`).
- [x] Two independent reference implementations — stop-and-wait and
      go-back-N, built directly on `frame`/`channel`, not reusing
      `transport` — with goodput measured against this transport's own
      selective-repeat + AIMD scheme across a loss sweep. Reported
      honestly: go-back-N collapses to worse than stop-and-wait the
      instant any reordering appears, and distrans is the *worst*
      performer at 20% loss, both real, unsmoothed findings.
- [x] Seeded chaos testing: arbitrary fault profiles, multiple
      concurrent clients, mid-stream profile changes, checked against
      an independently-written `ReferenceStore` by replaying the real
      server's exact execution order — replayable from a seed via the
      `distrans-chaos` binary. Found a real bug in a deliberately
      mutated `Delete` (confirming the oracle has teeth) and confirmed
      `near_partition` genuinely produces legitimate call failures
      (not silently never exercising that path).
- [x] The closing ADR answers the phase's research question directly,
      collecting every ticket's forced-vs-chosen finding with its
      evidence.

## Also found
- [x] The ticket-003 property's tick budget (separate from its retry
      budget) could also be exhausted at the harshest tested corner,
      leaving the connection still `Established` rather than `Failed`
      — same class of finding as ADR-003's original one, now covered
      completely.

Workload crate: 9 unit tests, 5 chaos tests, 1 goodput report. Full
workspace: 100 tests, all passing. See
`docs/design/decisions/ADR-006-integration-and-benchmarks.md`.
