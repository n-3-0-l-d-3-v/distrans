# DISTRANS — THE WIRE

> Reliable transport built over a deliberately hostile, unordered, lossy channel.

## Why "DISTRANS"

An actual in-universe device: a coded transmitter (often implanted in a bat) used to carry distorted messages across distance through a channel that is inherently hard to intercept and easy to lose. That is the exact job of this component — build something reliable and legible on top of a channel that offers no guarantees.

Part of **[ARRAKIS](https://github.com/n-3-0-l-d-3-v/arrakis)** — a constrained computing
ecosystem built by removing assumptions ordinary computers depend on. This
repository is developed standalone and mirrored into the combined ecosystem
repo commit-for-commit.

## Status

**Phase 5 — ACTIVE.** See [docs/design/WIRE.md](docs/design/WIRE.md)
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
