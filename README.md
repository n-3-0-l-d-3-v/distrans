# DISTRANS — THE WIRE

> Reliable transport built over a deliberately hostile, unordered, lossy channel.

## Why "DISTRANS"

An actual in-universe device: a coded transmitter (often implanted in a bat) used to carry distorted messages across distance through a channel that is inherently hard to intercept and easy to lose. That is the exact job of this component — build something reliable and legible on top of a channel that offers no guarantees.

Part of **[ARRAKIS](https://github.com/n-3-0-l-d-3-v/arrakis)** — a constrained computing
ecosystem built by removing assumptions ordinary computers depend on. This
repository is developed standalone and mirrored into the combined ecosystem
repo commit-for-commit.

## Status

**Phase 5 — COMPLETE.** All six tickets are done. See [docs/design/WIRE.md](docs/design/WIRE.md)
for the full layer map (hostile channel -> framing -> transport -> flow
control -> RPC) and the simulation model (virtual tick time, seeded
hostility).

**Ticket 001 (hostile channel) is done.** `crates/channel`: an
event-driven, virtual-tick-time datagram channel with seeded fault
injection (loss, duplication, reordering, corruption, truncation) plus
scripted per-datagram overrides for pinning one exact corner case. Faults
are decided at send time from a hand-rolled `SplitMix64` (no `rand`
dependency, so a recorded seed survives a dependency bump), so a whole
run replays byte-for-byte from `(seed, profile)` — proven, not assumed,
for 256 arbitrary profiles, and checked statistically that each fault's
observed rate matches its configured probability. 21 unit tests, 3
property tests, 1 statistical test, 1 no-wall-clock guard test. See
[ADR-001](docs/design/decisions/ADR-001-hostile-channel.md).

**Ticket 002 (framing and integrity) is done.** `crates/frame`: a fixed
12-byte binary header plus a CRC-32C trailer over header+payload. Checks
run cheapest-first (length, then version, then the checksum scan), and
every rejection reason is its own error variant. Property-tested (1024
cases each): round-trips for arbitrary frames, every single-bit and
odd-count multi-bit flip is either caught or harmlessly decodes back to
the original, and decoding arbitrary garbage or a truncated prefix never
panics. Fed real frames through a real hostile `Channel` at 40%
corruption / 30% truncation across 40 seeds to prove the two crates
actually compose, not just pass their own tests in isolation. See
[ADR-002](docs/design/decisions/ADR-002-framing-and-integrity.md).

**Ticket 003 (reliable, ordered transport) is done.** `crates/transport`:
a real connection state machine over the hostile channel — 3-way
handshake, byte-offset sequencing, cumulative + selective ACKs, a
selective-repeat receive buffer, adaptive (Jacobson/Karels) RTO with
Karn's algorithm, graceful teardown. Unlike TCP, SYN/FIN don't consume
sequence-number space; that's a deliberate, examined choice, not an
oversight (see the ADR for the trade-off). Getting the end-to-end tests
to pass surfaced two real state-transition races — a `Closed` connection
ignored a lingering retransmitted FIN, and the handshake required a bare
completing ACK that a data-sending client's own retry never resent — both
fixed and pinned with deterministic regression tests after discovering
the original statistical seed-loop test that first found them doesn't
reliably re-catch either on its own. A third finding, in the test itself
rather than the transport: the headline property's "always fully
completes" assertion was stronger than bounded retries can honestly
promise against the harsher end of its own fault-probability ranges;
fixed to assert what's actually always true (delivered data is a clean
prefix of what was sent) rather than loosening the ranges to hide it. See
[ADR-003](docs/design/decisions/ADR-003-reliable-transport.md).

**Ticket 004 (flow and congestion control) is done.** Every segment,
including the handshake, now carries the sender's currently free receive
capacity in bytes; admission is capped by `min(peer window, congestion
window)`. Congestion control is RFC 5681 AIMD: slow start, congestion
avoidance, multiplicative decrease on an RTO, fast retransmit on 3
duplicate ACKs. Its own headline property test was wrong twice before it
was right: "in-flight never exceeds the window at every tick" failed on
a clean channel because an RTO's `cwnd` reset doesn't retroactively
shrink data already in flight (exactly like real TCP), and a fixed
post-retransmission grace period — the first attempted fix — was also
wrong, since recovery time depends on RTO/backoff magnitude, not a
constant tick count. Fixed by testing the actual claim (admission
respects the window when it runs) directly and deterministically instead
of through a confounded multi-tick simulation. A goodput-vs-loss
measurement across window sizes found something genuinely worth
reporting: quadrupling the receive window bought nothing once the
congestion window, not the receive window, was the real bottleneck. See
[ADR-004](docs/design/decisions/ADR-004-flow-and-congestion-control.md).

**Ticket 005 (RPC with idempotent retry) is done.** `crates/rpc`:
`transport::Connection` is a byte stream with no message boundaries, so
a small length-prefixed reassembly layer sits underneath request/
response framing. The client retries independently of the connection's
own reliable-delivery retries — not redundant, since a client's patience
and the connection's retry budget are different things, and the harness
deliberately uses a short, fixed retry deadline that forces frequent
retries even on a healthy connection, exactly what an impatient real
client does. That means the server's dedup table isn't a defensive
nicety — it's what makes retrying safe at all, proven end to end: for
arbitrary calls and fault profiles, every request's handler runs exactly
once, and every response the client sees for one request is identical
across however many times it was retried. See
[ADR-005](docs/design/decisions/ADR-005-rpc.md).

**Ticket 006 (integration and benchmarks) is done — Phase 5 (THE WIRE)
is complete.** `crates/workload`: a real get/put/delete key-value
service on `rpc`, driven by several simulated clients through five named
hostile profiles, checked by seeded chaos testing that replays the real
server's exact execution order against an independently-written
reference store (which caught a deliberately reintroduced `Delete` bug
immediately, confirming the oracle has teeth). Two from-scratch reference
ARQ schemes — stop-and-wait and go-back-N, built directly on
`frame`/`channel`, not reusing `transport` — give a real, unsmoothed
comparison: go-back-N pipelines beautifully on an ideal channel (7.7×
stop-and-wait) but collapses to *worse than stop-and-wait* the instant
any reordering appears at all, because its receiver discards anything
out of order exactly as if it were lost — real, measured justification
for this transport's own selective-repeat design. distrans dominates at
low-to-moderate loss but is the *worst* performer at 20% loss, where AIMD's
multiplicative decrease repeatedly collapses the congestion window faster
than slow start can recover it — reported honestly, not hidden. The
closing ADR answers the phase's research question ("how much of TCP is
forced by physics versus convention?") by collecting every ticket's own
forced-vs-chosen finding. See
[ADR-006](docs/design/decisions/ADR-006-integration-and-benchmarks.md).

See [tickets/](tickets/) for the live phase-by-phase ticket board and
[docs/design/](docs/design/) for constraints, invariants and architecture
decision records.

## The constraint

The underlying channel provides no ordering, no reliable delivery, no framing, no integrity, and duplicates or corrupts datagrams. All of that must be built at this layer.

## What the constraint forces

Framing, checksums/integrity, sequence numbers, ACKs, retransmission, selective repeat, flow control, and RPC with idempotency.

## Research question

> How much of TCP's design is forced by physics versus by historical convention?

## Sibling repositories

- [mentat](https://github.com/n-3-0-l-d-3-v/mentat) — THE MACHINE (COMPLETE)
- [chakobsa](https://github.com/n-3-0-l-d-3-v/chakobsa) — THE LANGUAGE (COMPLETE)
- [muaddib](https://github.com/n-3-0-l-d-3-v/muaddib) — THE KERNEL (COMPLETE)
- [sietch](https://github.com/n-3-0-l-d-3-v/sietch) — THE VAULT (COMPLETE)
- [choam](https://github.com/n-3-0-l-d-3-v/choam) — THE DATABASE (QUEUED)
- [landsraad](https://github.com/n-3-0-l-d-3-v/landsraad) — THE COLONY (QUEUED)
- [ghola](https://github.com/n-3-0-l-d-3-v/ghola) — THE HISTORY (QUEUED)
- [shai-hulud](https://github.com/n-3-0-l-d-3-v/shai-hulud) — THE ARTIFACT (STRETCH)

## Development

This is a real, tested, benchmarked systems component — not a demo. See
[docs/DEFINITION_OF_DONE.md](docs/DEFINITION_OF_DONE.md) for the acceptance
bar every piece of this repo must clear before it is considered complete.

```bash
cargo build
cargo test
cargo bench
```
