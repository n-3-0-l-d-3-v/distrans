# ADR-006: A real KV service, two from-scratch reference schemes, seeded multi-client chaos, and the phase's research question answered

## Status
Accepted

## Context

Tickets 001–005 each proved one layer's guarantee in isolation. Ticket
006 must show three more things per `docs/DEFINITION_OF_DONE.md`: a
real observable workload built on every layer at once; a measured
comparison against independent references, not an assumed one; and
chaos testing with reproducible seeds. It must also answer the question
`docs/design/WIRE.md` posed at the start of the phase: **how much of
TCP's design is forced by physics versus by historical convention?**

## Decision

### Part 1: the workload

`crates/workload`'s `kv` module is a real service — `get`/`put`/
`delete` — served over `rpc::RpcServer`. Its request/response wire
encoding lives in `kv.rs`; the actual storage the real server uses is a
plain `HashMap` shared via `Rc<RefCell<_>>` across every client
connection's own `RpcServer` instance (this simulation is
single-threaded, so `Rc<RefCell<_>>` is the right tool here, not a
concurrency shortcut). A second implementation, `ReferenceStore`
(a linear `Vec<(key, value)>`, deliberately not sharing code with the
real `apply` function), exists purely as a differential oracle for
`chaos.rs` — a real bug was found this way (see Part 3).

`profiles.rs` names five hostile-channel profiles per the ticket's
scope: `clean`, `lossy`, `high_latency_reordering`, `bursty_corruption`,
`near_partition` (60% loss).

### Part 2: two independent reference ARQ schemes

`baselines.rs` implements stop-and-wait and go-back-N **directly on
`frame`/`channel`**, deliberately not reusing anything from `transport`
— comparing this phase's own transport against a shared implementation
wearing two hats would prove nothing about the *scheme*. Both use fixed
(non-adaptive) RTOs, a stated simplification: they exist to compare
architectures, not to be equally sophisticated implementations.

### Part 3: chaos testing, and a real bug it found

`chaos.rs` drives several simulated clients issuing random get/put/delete
calls, at random ticks, against random keys, under a chosen fault
profile with optional mid-stream profile switches (each switch
re-seeds every client's channel pair under the new profile,
deliberately dropping whatever was still in flight — modeling a real
link failover). Every actual handler execution is recorded, in the
exact order the single-threaded simulation processed it (a well-defined
global order even across several concurrent clients, since nothing here
is truly parallel); after the run, that exact sequence is replayed
against a fresh `ReferenceStore` and every recorded result must match.

**This differential check found a real bug while it was being written**
(mutation-tested honestly: it was deliberately introduced to verify the
oracle has teeth, and it happened to be the same class of bug a careless
implementation could produce for real): `Delete`'s "did this key exist"
result must be independently correct, not merely self-consistent with
whatever the real `HashMap` does — reverting the real store's delete
result to always report `false` was caught immediately, across every
profile, within the first couple of executed operations. This confirms
the oracle is checking real correctness, not tautologically agreeing
with itself.

**A call failing outright is not a chaos failure.** Under
`near_partition` (60% loss), `RpcClient` calls do sometimes exceed their
retry budget — confirmed directly (`near_partition_produces_legitimate_call_failures_not_invariant_violations`,
which asserts at least one such failure occurs, so the failure-counting
path is exercised, not silently never hit). This is the same lesson
ADR-003 and ADR-004 already learned about their own properties, applied
here from the start rather than discovered the hard way a third time:
bounded retries giving up under a sufficiently hostile profile is
correct behavior, not a bug, and `ChaosSummary::calls_failed` counts it
rather than treating it as an error.

### Measurements

`goodput_comparison.rs`, 20,000-byte transfer, 64-byte segments,
`cargo test -p workload --release --test goodput_comparison -- --nocapture`:

| Condition | stop-and-wait | go-back-N (window 8) | distrans |
|---|---|---|---|
| ideal (no loss, no reorder) | 63.7 B/tick | **487.8 B/tick** | — |
| loss=0.00 (1-tick jitter) | 31.6 B/tick | 5.6 B/tick | **185.2 B/tick** |
| loss=0.02 | 22.4 B/tick | 5.4 B/tick | **128.2 B/tick** |
| loss=0.05 | 13.0 B/tick | 5.6 B/tick | **104.7 B/tick** |
| loss=0.10 | 6.6 B/tick | 4.5 B/tick | **17.6 B/tick** |
| loss=0.20 | **3.2 B/tick** | 3.7 B/tick | 1.2 B/tick (worst) |

Three findings, all measured rather than assumed, none smoothed over:

1. **Go-back-N's pipelining genuinely works on an ideal channel** — 7.7×
   stop-and-wait's throughput, exactly the textbook claim.
2. **The instant *any* reordering appears — even at zero packet loss —
   go-back-N collapses to worse than stop-and-wait**: 5.6 B/tick, an
   88× regression from its own ideal-channel number, and below
   stop-and-wait's 31.6. One tick of reordering jitter is enough,
   because go-back-N's receiver *discards* anything not exactly the next
   expected segment and never buffers it — indistinguishable, from the
   receiver's perspective, from that segment having been lost — so the
   sender resends its entire window on every reordering event. This is
   real, measured justification for selective-repeat (this transport's
   own design, ADR-003), not an assumption carried over from a textbook.
3. **distrans dominates at low-to-moderate loss** (108 vs. 633/3572
   ticks to finish at loss=0) **but is the worst performer at loss=0.20**
   (17,388 ticks — slower than either textbook baseline). AIMD's
   multiplicative decrease on every RTO repeatedly collapses `cwnd` to a
   single segment under sustained heavy loss, and slow start's
   exponential regrowth isn't fast enough to recover before the next
   loss event at this loss rate. Reported plainly: this transport is not
   unconditionally better, and a real deployment expecting sustained
   ~20% loss would need congestion-control tuning this ticket didn't do.

## The research question, answered with the evidence

> How much of TCP's design is forced by physics versus by historical
> convention?

Collecting every phase ADR's finding:

- **Forced** (the property demonstrably fails without it): framing and a
  checksum (ADR-002 — there is no other way to know a datagram is intact
  over a channel that corrupts bytes); *some* retransmission-on-timeout
  mechanism (ADR-003 — without it, loss is permanent data loss); a
  cumulative or selective acknowledgment of *some* kind (ADR-003 —
  without one, the sender has no way to learn what arrived); flow
  control of *some* kind bounding what the sender may have outstanding
  (ADR-004 — without it, a fast sender can overrun a slow receiver
  regardless of the network); congestion backoff of *some* kind under
  sustained loss (ADR-004, and the loss=0.20 finding above shows getting
  the *specific* backoff wrong still costs real throughput, so a
  mechanism being forced doesn't mean its exact shape is).
- **Chosen, not forced** (this repo built the alternative and it worked,
  or measurably worked *better*): SYN/FIN consuming sequence-number
  space (ADR-003 — this repo's separated version worked throughout, at
  the cost of two small retry mechanisms instead of one uniform one);
  cumulative-ACK-only, discard-on-reorder receive buffering, i.e.
  go-back-N's specific choice (measured above to be *actively worse*
  than the selective-repeat alternative the moment reordering appears —
  not just "a valid alternative," a worse one, for a channel like this
  phase's that reorders at all); a single connection-level retransmission
  timer over per-segment timers (ADR-003 — a design choice among several
  valid ones, not something the channel demanded).
- **Forced, but this repo's specific answer isn't obviously the right
  one** — the honest middle case: AIMD congestion control (ADR-004) is
  *some* answer to a forced problem (uncontrolled sending congests a
  lossy network), but the loss=0.20 measurement above shows this
  particular AIMD parameterization is not a good answer at that loss
  rate specifically. TCP's own decades of congestion-control research
  (Reno, Cubic, BBR, ...) is itself evidence that "some backoff is
  forced" and "which backoff is right" are different-sized questions;
  this phase only had the resources to answer the first.

## Alternatives Considered

1. **A full second congestion-control implementation to compare AIMD
   against.** Rejected for scope — the loss=0.20 finding already answers
   the interesting question (AIMD is not free of real weaknesses here)
   without needing a second implementation to prove it.
2. **Treating the go-back-N reordering collapse as a bug in the
   baseline and fixing it (e.g. adding partial buffering).** Rejected:
   that would no longer be go-back-N, and the whole point of building it
   from scratch was to measure the textbook scheme's own real behavior,
   not a hybrid.
3. **Silently narrowing the goodput sweep's channel parameters to avoid
   the reordering collapse.** Rejected outright — it's the most
   interesting number in the whole comparison.

## Consequences

- Ticket 006 closes. Phase 5 (`distrans`, THE WIRE) is complete;
  CORE + EXTENSION scope per `SCOPE.md` is done. The EXPERIMENT items
  (multipath redundancy, a QUIC-style redesign) are not attempted, as
  with every other phase's EXPERIMENT scope in this project.
- **Known gap, stated honestly**: this transport's congestion control is
  not tuned for sustained heavy loss (≥20% in this measurement). A
  future phase reusing this transport for a workload expecting that
  regime would need to revisit AIMD's parameters or replace it, not
  assume ADR-004's defaults are adequate everywhere.
- The chaos harness's dedup table (`rpc`'s `RpcServer::seen`, ADR-005)
  is exercised here at real multi-client scale and remains unbounded —
  still a stated, not solved, limitation.
