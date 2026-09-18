---
status: done
phase: 5
---

# 003 — Reliable, ordered transport

The core promise of the whole repo: turn `channel` + `frame` into a byte
stream where the receiver sees exactly what the sender sent, in order,
exactly once, despite loss, duplication, reordering and corruption.

## Scope
- [x] Connection establishment (3-way handshake) and graceful teardown
      (FIN/fin_ack), over the hostile channel.
- [x] Sequence numbers (byte offsets); cumulative ACKs; selective ACKs
      (up to 4 merged ranges) so the sender learns exactly what arrived
      beyond the cumulative point.
- [x] A selective-repeat receive buffer: out-of-order segments are held
      and delivered in order once the gap fills.
- [x] Duplicate suppression at the receiver.
- [x] Adaptive retransmission timeout in ticks (Jacobson/Karels, Karn's
      algorithm), exponential backoff on repeated timeout, a single
      connection-level timer keyed off the oldest unacknowledged byte.
- [x] Bounded-retry give-up: `State::Failed(FailureReason::TooManyRetries)`
      under total/permanent loss, verified within a bounded tick budget.
- [x] Property test: arbitrary payload data and an arbitrary fault
      profile short of total loss delivers byte-for-byte, in order, no
      duplication — differentially checked against the sent bytes
      themselves (48 cases, real `channel::Channel`s both directions).
- [x] Property/direct test: total permanent loss fails within bounded
      ticks, not forever.

## Two real bugs found and fixed (see ADR-003)
- [x] A `Closed` connection ignored a lingering retransmitted FIN whose
      own ack was lost, so the peer retried forever — fixed (a `Closed`
      connection still answers a FIN with a fresh ack).
- [x] The handshake required a bare, payload-free completing ACK; if lost,
      the server was stuck even though the client (having sent real data
      immediately) believed the connection was fine — fixed (any
      ACK-flagged segment completes the handshake).
- [x] Both pinned by deterministic unit tests after discovering the
      original statistical seed-loop test doesn't reliably re-catch
      either regression on its own.
- [x] A third finding: the headline property test's own assertion (full
      completion, always) was too strong — bounded retries legitimately
      give up at the harsher end of the tested fault-probability ranges.
      Fixed to assert the transport's actual guarantee (an exact prefix
      of what was sent, always; full completion when the retry budget
      allows it) rather than loosening the fault ranges to hide it.

25 unit tests (11 segment/RTO + 14 connection), 7 end-to-end reliability
tests including one property test. See
`docs/design/decisions/ADR-003-reliable-transport.md`.
