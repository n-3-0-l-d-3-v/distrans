---
status: open
phase: 5
---

# 004 — Flow and congestion control

Prevent a fast sender from overrunning a slow receiver, and prevent a
sender from overrunning the (simulated) network itself.

## Scope
- Receiver-advertised window: the receiver tells the sender how much
  unacknowledged data it can currently buffer; the sender never exceeds
  it.
- A congestion window with AIMD (additive increase, multiplicative
  decrease on loss), slow start, and fast retransmit on a small number of
  duplicate ACKs.
- Property test: for an arbitrary receiver buffer size and arbitrary
  fault profile, the sender's unacknowledged-bytes-in-flight never
  exceeds `min(receiver window, congestion window)` at any tick.
- Benchmark: goodput vs. induced loss rate, at several buffer sizes —
  numbers, not assumptions, comparable against ticket 006's baselines.

Not started. Depends on ticket 003.
