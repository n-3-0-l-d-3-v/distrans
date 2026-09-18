# Scope — distrans

## CORE (required for this repo to be considered complete at all)
- A simulated hostile channel: virtual (tick-based) time, seeded fault
  injection (loss, duplication, reorder via delay, corruption,
  truncation), scripted adversarial schedules, reproducible from a seed
  (ticket 001).
- Framing and integrity: a versioned binary frame header plus a CRC-32C
  trailer; decoding never panics and always distinguishes a well-formed
  frame from a corrupted or truncated one (ticket 002).
- Reliable, ordered, exactly-once-delivered byte transport over the
  hostile channel: sequence numbers, cumulative + selective ACKs,
  selective-repeat receive buffering, duplicate suppression, adaptive
  retransmission timeout, connection establishment/teardown (ticket 003).

## EXTENSION (required for full integration into the combined ecosystem)
- Flow control (receiver-advertised window) and congestion control (AIMD
  congestion window, fast retransmit) layered on the transport
  (ticket 004).
- RPC over the transport with client retry and server-side idempotency
  (at-most-once execution via a dedup table) (ticket 005).
- A real workload (an RPC key-value service) driven through hostile
  channel profiles, goodput-vs-loss benchmarks against stop-and-wait and
  go-back-N reference implementations, seeded chaos testing, and the
  research-question write-up (ticket 006).

## EXPERIMENT (only attempted once CORE + EXTENSION are healthy)
- A path-diversity/multipath extension: the same connection spread over
  several simulated hostile channels with different fault profiles, to
  see whether reliability is cheaper to buy with redundancy than with
  retransmission alone.
- A userspace-QUIC-style redesign (stream multiplexing over one
  connection, 0-RTT-ish reconnection) to see how much of TCP's specific
  shape (vs. reliable-transport-in-general) survives being rebuilt from
  scratch with hindsight.
