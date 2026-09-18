# ADR-004: Byte-based receiver window on every segment, RFC 5681 AIMD, and a property test corrected twice by the same underlying fact

## Status
Accepted

## Context

Ticket 003's transport had a fixed `max_in_flight_segments` cap standing
in for real flow control, with no notion of the peer's actual buffer
capacity and no reaction to loss beyond "retransmit and back off."
Ticket 004 replaces that with a receiver-advertised window (so a sender
never overruns what the peer can actually buffer) and a congestion
window (so a sender backs off when the *network*, not just this one
peer, seems to be struggling) — the two distinct pressures real flow
control balances.

## Decision

**Every segment carries the sender's current receive window, including
the handshake.** `SegmentHeader` gained a `window: u32` field
(bytes free right now: `recv_window_capacity` minus bytes held in the
out-of-order buffer plus bytes delivered-but-not-yet-drained by the
application — the latter modeling a real socket's receive buffer, not
just the reassembly gap). Ticket 003's SYN/SYN-ACK/handshake-ACK
originally carried an empty payload as a special case (no header at
all, since nothing needed reporting yet); that special case is removed
here — every frame, including the handshake's, now carries a header, so
the peer's flow control has a real number from the very first round
trip rather than an assumed default until the first data ack. This also
simplified `on_datagram`, which no longer needs an
empty-payload-means-no-header branch.

**Admission is `min(peer_window, cwnd)` minus bytes already in flight,
with one deliberate escape hatch.** `try_send_more` computes this once
per loop iteration (not once per call — bytes admitted in this same
call reduce room for the next). If nothing is currently in flight, at
least 1 byte is always allowed out even when the window is smaller than
that — otherwise an advertised window of zero (or smaller than one
segment) would stall the connection permanently with no way to ever
probe it again. This is a known, stated simplification: real TCP solves
zero-window stalls with a persist timer that periodically re-probes;
this repo doesn't implement one (see Consequences).

**Congestion control is RFC 5681's AIMD, in bytes, with Karn-consistent
sampling already reused from ADR-003.** Slow start (`cwnd < ssthresh`):
each cumulative ack grows `cwnd` by the number of bytes it newly
acknowledged (the byte-counted equivalent of "double every RTT").
Congestion avoidance (`cwnd >= ssthresh`): grows by
`segment_size² / cwnd` per ack, the standard ~one-segment-per-RTT
approximation. Two distinct loss signals, two distinct (both standard)
responses: an RTO timeout is multiplicative decrease (`ssthresh =
max(cwnd/2, 2×segment)`, `cwnd` reset to exactly one segment — the
network may be badly congested); three consecutive duplicate ACKs
(fast retransmit) is gentler (`ssthresh` the same way, but `cwnd` set
*to* `ssthresh` rather than collapsed to one segment, since duplicate
ACKs mean segments are still getting through past the lost one).

## A property test corrected twice by the same underlying fact

The first property tried was the obvious one: "at every tick,
`in_flight_bytes() <= effective_window()`." It failed immediately, on a
*clean* channel with zero loss — investigation showed no admission bug:
an RTO resets `cwnd` to one segment, but the bytes sent *before* the
timeout, now awaiting retransmission, don't retroactively shrink to fit.
They're still genuinely outstanding. Real TCP works identically — a
window or congestion-window reduction is a signal for *future*
admission decisions, never a forced recall of data already sent. This
mirrors ADR-003's finding about its own property test almost exactly:
an assertion that sounded like the invariant was actually stronger than
what the protocol can honestly promise.

The fix attempted first — skip the check for a fixed number of ticks
after any retransmission — was *also* wrong, and for an informative
reason: recovery time depends on the RTO magnitude (which includes
exponential backoff) and how long it takes an ack to arrive, neither of
which is bounded by a small fixed tick count. A harsher fault profile
kept re-triggering retransmissions faster than the grace period could
expire, and the "recovered" state it was supposedly waiting for
sometimes just... took longer than a few ticks, for entirely legitimate
reasons.

**The actual fix: test the right thing with the right tool.** The
guarantee ticket 004 asks for is about *admission decisions*, not about
an instantaneous snapshot confounded by history. That's directly and
deterministically testable without any multi-tick simulation:
`a_single_send_never_admits_more_than_the_effective_window` and
`a_smaller_advertised_window_caps_further_admission` call `send()` once,
with no timer running, and check the invariant immediately — no
confound possible, no statistical flakiness, and each is mutation-checked
(reverting admission's window check, or the RTO's `cwnd` reset, or fast
retransmit's trigger, fails its corresponding test). The end-to-end
statistical property was removed rather than patched a second time,
since a black-box, multi-tick simulation cannot distinguish "admission
ignored the window" from "the window legitimately changed after
admission already happened" without reconstructing history the test
itself doesn't have — a white-box, single-call test can, trivially.

## Measurements

`report_goodput_vs_loss_and_window_size` (a deterministic, clean-channel
except for the stated loss rate, 20,000-byte transfer, `max_retries=30`,
`max_rto=1000`):

| Window | loss=0 | loss=0.05 | loss=0.10 | loss=0.20 |
|---|---|---|---|---|
| 128 B (2 segments) | 16.22 B/tick | 13.60 B/tick | 5.23 B/tick | 0.25 B/tick, **never finished in 60,000 ticks** |
| 512 B (8 segments) | 77.22 B/tick | 52.49 B/tick | 20.62 B/tick | 0.47 B/tick (finished) |
| 2048 B (32 segments) | 75.76 B/tick | 53.19 B/tick | 18.99 B/tick | 0.45 B/tick (finished) |

**Reported honestly, without smoothing over the surprising part**: going
from a 512 B to a 2048 B window (4×) produces **no measurable
improvement**, and is very slightly *worse* at every loss rate tested.
Once the congestion window — not the receiver's advertised capacity —
is the binding constraint (which it is here almost immediately, since
`cwnd` starts small and AIMD governs growth regardless of how large the
receiver says it can buffer), a bigger receive window buys nothing; the
bottleneck moved elsewhere and enlarging the wrong knob doesn't help. At
128 B, the window itself *is* the bottleneck (only 2 segments can ever
be in flight, so even a perfectly healthy congestion window can't help),
and at 20% loss it makes the difference between finishing and not
finishing within a generous tick budget at all — a receive window an
application actually needs headroom in matters far more than one that's
merely "big."

## Alternatives Considered

1. **Keep the empty-payload handshake special case, add window only to
   data/ack segments.** Rejected: the peer would run one RTT blind
   (assuming a conservative default window) before learning the real
   one, and removing the special case simplified `on_datagram` besides.
2. **Per-segment RTO-driven cwnd cuts (cut on every timed-out segment
   individually).** Rejected: RFC 5681's per-*event* (not per-segment)
   cut is the standard, and ticket 003 already made retransmission an
   all-non-sacked-segments event, so a per-event cwnd cut is the natural
   fit.
3. **A zero-window persist timer.** Deferred, not implemented — see
   Consequences.

## Consequences

- Ticket 004 closes.
- **Known limitation, stated plainly**: no zero-window persist/probe
  timer. If a peer ever advertises a window of exactly 0, the "at least
  1 byte when nothing is in flight" rule still lets the sender probe
  once, but there's no periodic re-probe beyond what the normal
  RTO/retransmission machinery happens to provide, and no dedicated
  "window update" notification when a stalled receiver frees space
  outside of the normal ack/retransmission flow. Not exercised by this
  ticket's tests, and not needed by ticket 005's RPC workload's message
  sizes, but a real deployment approaching a persistently tiny window
  would need it.
- `Config::max_in_flight_segments` is gone; anything downstream
  (ticket 005's RPC layer, ticket 006's workload) configures
  `recv_window_capacity` instead.
- The goodput-vs-loss-and-window data above is this ticket's own
  contribution; ticket 006 additionally compares this transport's
  *scheme* (selective-repeat + AIMD) against stop-and-wait and
  go-back-N baselines, which is a different question (architecture, not
  parameter tuning) from what's measured here.
