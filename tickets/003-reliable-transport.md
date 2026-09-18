---
status: open
phase: 5
---

# 003 — Reliable, ordered transport

The core promise of the whole repo: turn `channel` + `frame` into a byte
stream where the receiver sees exactly what the sender sent, in order,
exactly once, despite loss, duplication, reordering and corruption.

## Scope
- Connection establishment (handshake) and teardown, over the hostile
  channel.
- Sequence numbers; cumulative ACKs; selective ACKs (SACK) so the sender
  learns exactly which segments beyond the cumulative point arrived.
- A selective-repeat receive buffer: out-of-order segments are held and
  delivered in order once the gap fills, not discarded.
- Duplicate suppression at the receiver (a duplicate or a retransmitted-
  but-already-acked segment is dropped, not redelivered).
- Adaptive retransmission timeout in ticks (Jacobson/Karels RTT
  estimation over virtual time), with exponential backoff on repeated
  timeout.
- A bounded-retry give-up: under total partition, the connection
  reports a typed failure rather than retrying forever.
- Property test: for arbitrary payload data and an arbitrary fault
  profile short of total/permanent loss, the receiver's delivered byte
  stream equals the sender's, byte for byte, in order, with no
  duplication — for many random (seed, profile) pairs, differentially
  checked against a plain in-memory copy as a correctness oracle.
- Property test: under total, permanent packet loss, the connection
  fails within a bounded number of retries/ticks rather than hanging.

Not started. Depends on tickets 001, 002.
