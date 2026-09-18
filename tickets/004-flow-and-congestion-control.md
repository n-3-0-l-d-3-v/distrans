---
status: done
phase: 5
---

# 004 — Flow and congestion control

Prevent a fast sender from overrunning a slow receiver, and prevent a
sender from overrunning the (simulated) network itself.

## Scope
- [x] Receiver-advertised window: every segment (including the
      handshake) carries the sender's currently free receive-buffer
      capacity in bytes; the peer's admission control never exceeds it.
- [x] A congestion window with AIMD (RFC 5681): slow start, congestion
      avoidance, multiplicative decrease (halved `ssthresh`, `cwnd`
      reset to one segment) on an RTO timeout, and fast retransmit
      (resend the oldest unacked segment immediately, `cwnd` set to the
      new `ssthresh`) on 3 consecutive duplicate ACKs.
- [x] Property/direct test: admission never lets more data become
      unacknowledged than `min(receiver window, congestion window)`
      allowed at the moment of admission — tested deterministically
      (see below for why the end-to-end statistical version was wrong).
- [x] Benchmark: goodput vs. induced loss rate, at three receive-window
      sizes — numbers in ADR-004, including a genuinely surprising one
      (going from a 512 B to a 2048 B window bought nothing, since the
      congestion window, not the receive window, was already the
      binding constraint at every loss rate tested).

## A property test found wrong twice, fixed correctly the third time
- [x] "In-flight bytes never exceed the effective window at every tick"
      failed on a clean channel — not a bug: an RTO's `cwnd` reset
      doesn't retroactively shrink data already in flight, exactly like
      real TCP.
- [x] A fixed post-retransmission grace period (the first attempted fix)
      was also wrong: recovery time depends on RTO/backoff magnitude,
      not a constant number of ticks.
- [x] Fixed by testing the actual claim (admission respects the window
      at the moment it runs) with a direct, deterministic unit test
      instead of a confounded multi-tick simulation. Mutation-checked.

18 unit tests (4 new: admission-respects-window ×2, RTO resets `cwnd`,
fast retransmit fires), 8 end-to-end reliability tests (one removed,
replaced by the direct tests above; one new goodput report). See
`docs/design/decisions/ADR-004-flow-and-congestion-control.md`.
