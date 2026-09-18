# THE WIRE — architecture (Phase 5)

## Overview

DISTRANS builds reliable, ordered, flow-controlled, integrity-checked
communication, and RPC on top of it, over a channel that guarantees
**nothing**. Per `docs/design/CONSTRAINTS.md`, the underlying channel may
drop, duplicate, reorder, delay and corrupt datagrams, and it has no notion
of framing. Every property a conventional application takes for granted
from TCP is rebuilt here, one layer at a time, and each layer's guarantee
is proven against a channel that is actively trying to break it.

## The simulation model: virtual time, seeded hostility

Like mentat simulates a machine and muaddib simulates a kernel, distrans
runs over a **simulated network**, not real sockets. Two decisions follow
from the Definition of Done's reproducibility requirement:

1. **Virtual time.** Time is a `Tick` counter advanced only by the
   simulator. Retransmission timeouts, RTT estimates and delays are all
   measured in ticks. No layer reads physical time, so a whole run, every
   timeout firing included, is a pure function of its inputs.
2. **Seeded hostility.** Every fault the channel injects (loss,
   duplication, reordering via random delay, bit-flip corruption,
   truncation) comes from a seeded PRNG under a configurable fault
   profile. Any failure replays exactly from `(seed, profile, workload)`.
   Scripted adversarial schedules ("drop exactly the 3rd retransmission of
   segment 7") are also supported, for deterministic regression tests of
   specific corner cases.

This is a deliberate trade: no real-network timing noise, in exchange
for exhaustive, replayable exploration of fault interleavings. That
exploration is the part real-network testing can't do.

## Layer map

```text
crates/channel    -- the hostile datagram channel: virtual-time event
                     queue, seeded fault injection (loss, duplication,
                     delay/reorder, corruption, truncation), scripted
                     adversarial schedules, per-fault statistics (ticket 001)
crates/frame      -- framing + integrity: a versioned binary header and
                     a CRC-32C trailer; decoding rejects every malformed or
                     corrupted datagram with a typed error, never a panic
                     and never a silently wrong frame (ticket 002)
crates/transport  -- reliable ordered delivery: sequence numbers
                     (byte offsets; SYN/FIN do NOT consume sequence
                     space, unlike TCP — see ADR-003), cumulative +
                     selective ACKs, selective-repeat receive buffer,
                     duplicate suppression, a single connection-level
                     retransmission timer with adaptive (Jacobson/Karels)
                     RTO in ticks, 3-way handshake and graceful teardown
                     (ticket 003, DONE)
                  -- flow and congestion control: receiver-advertised
                     window on every segment (incl. handshake), RFC 5681
                     AIMD congestion window, fast retransmit on 3 dup
                     ACKs (ticket 004, DONE)
crates/rpc        -- request/response over transport: request ids,
                     client retry, server-side idempotency (at-most-once
                     execution) via a dedup table (ticket 005)
crates/workload   -- closing ticket: an RPC key-value service driven
                     through hostile profiles, goodput-vs-loss benchmarks
                     against stop-and-wait and go-back-N baselines, seeded
                     chaos, and the research-question write-up (ticket 006)
```

## Invariants each layer must prove (not just assert)

- **channel**: fault injection matches its profile statistically, and a
  run is byte-for-byte reproducible from its seed.
- **frame**: `decode(encode(f)) == f` for every frame; every corrupted or
  truncated datagram is either rejected or (only with CRC-32C's collision
  probability) accepted. Never a panic. All single-, double- and odd-count
  bit errors and all bursts up to 32 bits are detected, which CRC-32C
  guarantees for these frame sizes; this is property-tested, not just
  claimed.
- **transport**: whatever the receiver delivers is always an exact,
  gap-free, unreordered, uncorrupted prefix of what the sender sent —
  proven for arbitrary data and arbitrary fault profiles, including ones
  hostile enough that bounded retries give up before the whole transfer
  completes (a real, examined finding — see ADR-003 — not merely a
  weaker claim assumed for convenience). Under total, permanent loss,
  the connection reports failure within a bounded number of ticks rather
  than hanging.
- **flow control**: admission never lets more data become unacknowledged
  than `min(peer window, congestion window)` allowed *at the moment of
  admission* — proven directly, not by an instantaneous snapshot at
  arbitrary later times, since a legitimate window/cwnd reduction never
  retroactively shrinks data already in flight (a real finding, not an
  assumption — see ADR-004).
- **rpc**: under retries and duplicated requests, every request's handler
  executes at most once, and every completed call's response is the one
  that execution produced.

## Research question

> How much of TCP's design is forced by physics versus by historical
> convention?

Each layer's ADR records which TCP mechanism the hostile channel *forced*
(the property fails without it, demonstrably) versus which was a choice
(an alternative works as well, measured). The closing ticket's ADR
collects the answer.
