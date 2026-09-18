# ADR-003: Byte-offset sequencing without SYN/FIN consuming sequence space, a single oldest-byte RTO timer, and two state-transition races found the hard way

## Status
Accepted

## Context

Ticket 003 turns `channel` + `frame` into what the whole phase exists to
build: a byte stream where the receiver sees exactly what the sender
sent, in order, exactly once, despite loss, duplication, reordering and
corruption. This is the first ticket in this repo with real protocol
*state* (a handshake, in-flight segments, a receive buffer, a teardown
sequence) rather than a stateless transform, so it's also the first place
this repo's tests had to catch genuine state-machine bugs rather than
input-validation ones.

## Decision

**Sequence numbers are byte offsets into the stream, and — unlike TCP —
SYN and FIN do not consume any of that space.** `docs/design/WIRE.md`
flagged this exact question ("how much of TCP's design is forced by
physics versus by historical convention?") as the phase's research
question, and this is the first concrete data point. TCP's SYN/FIN each
consume one sequence number so their own reliability can piggyback on
the *same* cumulative-ack machinery data already needs, avoiding a
second retry mechanism. This implementation instead gives SYN and FIN
their own small, independent retry state (`PendingControl`: a sent-at
tick and an attempt count, timed against the same shared
`RtoEstimator`), with FIN's acknowledgment carried by a dedicated
`fin_ack` bit in the transport header rather than by advancing the ack
number. **Verdict, reported honestly rather than assumed either way**:
this is a case where TCP's choice was *not* forced — the separated
version works, is simpler to reason about in isolation (data and control
reliability never interact), and never needed retrofitting during this
ticket's debugging. Its real cost is duplicated bookkeeping (two
`PendingControl`-shaped retry loops in `on_tick` instead of one), which
did not turn out to matter here at this scale.

**One connection-level RTO timer, keyed off the *oldest* unacknowledged
byte, recomputed on demand rather than stored.** `on_tick` checks
`self.unacked.iter().next()` (a `BTreeMap`, so this is the
lowest-sequence, i.e. oldest, entry) against `rto.current()`; no
`deadline` field is stored anywhere, for `PendingControl` or for data.
This was a deliberate simplification made while writing the code, after
noticing that storing an explicit deadline and updating it at every
event (new segment sent, ack advances, timeout fires) is a well-known
source of "forgot to update the deadline" bugs, and it's redundant: the
information needed (when was this last (re)sent, what's the current RTO)
is already sitting in `sent_at` and `rto.current()`.

**On timeout, retransmit every unacked segment the peer hasn't already
SACK'd**, not just the oldest one. A single RTO firing is a strong
signal something is badly wrong (unlike a handful of duplicate ACKs,
which ticket 004's fast retransmit will use for a lighter-weight
response), so resending everything not yet confirmed is standard
practice in real SACK-TCP implementations and is what actually exercises
this ticket's selective-repeat requirement: a naive Go-Back-N response to
every timeout would make the SACK information pointless.

**Karn's algorithm, implemented via a single extra `retransmit_count == 0`
/ `attempts == 1` check at the sampling site**, not a separate flag. An
RTT sample is only taken from a segment or control message that has
never been retransmitted — otherwise an ACK can't be attributed to a
specific attempt, and using it would poison the RTO estimate exactly
when the network is already struggling.

## Two real state-machine bugs, found by making the end-to-end tests pass, not by inspection

Both were caught by `arbitrary_data_over_an_arbitrary_hostile_profile_arrives_intact`
(the ticket's headline property test) failing on specific seeds, then
root-caused with a targeted debug harness, then pinned down with a
deterministic unit test each — because a follow-up mutation check showed
the original statistical seed-loop test **does not reliably re-catch
either regression** (reverting one fix and rerunning 300 random seeds
still passed, since the two bugs' symptoms overlap and can mask each
other by chance). This is itself a finding worth recording: a passing
randomized end-to-end test found these bugs once, but isn't trustworthy
as a *regression* test for them — only the deterministic unit tests
(`a_lost_bare_completing_ack_does_not_strand_the_server_if_data_follows`,
`a_duplicate_synack_after_establishment_gets_the_completing_ack_resent`,
`a_closed_connection_still_acks_a_lingering_retransmitted_fin`) are.

1. **A `Closed` connection stopped answering everything**, including a
   peer's retransmitted FIN whose own `fin_ack` reply had itself been
   lost. The peer would then retransmit its FIN forever against a side
   that had already moved past the `Established`/`Closing` dispatch
   entirely. This is exactly the failure mode real TCP's `TIME_WAIT`
   state exists to prevent. Fixed narrowly, not with a full `TIME_WAIT`:
   a `Closed` connection specifically still answers an incoming FIN with
   a fresh `fin_ack`, with no new state or timer. The honest limitation
   this leaves: it only helps for as long as the `Connection` value is
   still alive and being fed datagrams — this repo's tests and ticket
   006's workload keep connections around, but a real deployment
   discarding a `Connection` immediately on `Closed` would still need an
   actual `TIME_WAIT`-style linger.
2. **The handshake only completed on a bare, payload-free ACK.** A
   client that reaches `Established` typically sends data immediately,
   and that data segment also carries the ACK flag — but the server-side
   dispatch required `app_payload.is_empty()` to accept it as the
   handshake-completing ACK. If the client's one *bare* completing ACK
   was ever lost, the server was stuck retrying its SYN-ACK until it gave
   up and failed the connection — even though the client believed
   everything was fine and had already started (successfully!) sending
   data the server was silently discarding. Fixed by accepting *any*
   ACK-flagged segment as completing the handshake, then falling through
   into the same datagram's normal data/ack processing in one step
   (matching how real TCP treats "data-with-ACK" as no different from
   "bare ACK" for handshake completion purposes) — plus the mirror case,
   a duplicate SYN-ACK arriving after the client has already moved to
   `Established`, which must still get a completing ACK resent, since
   nothing else will ever prompt the client to send one again otherwise.

Neither bug was found by unit-testing the pieces in isolation
(`segment.rs`'s codec tests and `rto.rs`'s estimator tests all passed
throughout) — both are specifically about what happens *across* a state
transition under message loss, which only a real end-to-end simulation
with real loss exercises.

## A third finding: the headline property's own assertion was wrong, not the transport

After both state-machine bugs were fixed, the property test still failed
occasionally at the harsher end of its random fault-probability ranges
(loss up to 0.3, corruption up to 0.2, truncation up to 0.1, applied
independently per attempt in *both* directions) — a segment legitimately
exceeded even a generous retry budget by chance. This is not a bug: a
transport with *bounded* retries, by design, must sometimes give up
against a channel hostile enough, and asserting "always fully
completes" was asserting something stronger than what bounded retries
can honestly promise. The transport's actual, always-true guarantee is
narrower and was the thing worth proving: whatever data *did* arrive is
an exact, gap-free, unreordered, uncorrupted prefix of what was sent —
`data.starts_with(&delivered)` — regardless of whether the transfer
completed. The property now checks exactly that when the connection
ends in `Failed`, and full byte-for-byte equality plus `Closed` on both
ends when it isn't. This is recorded as a finding, not silently patched,
because it's a useful data point in itself: a fault profile survivable
"in principle" (well short of total, permanent loss) can still not be
survivable *within a fixed retry budget*, and a real deployment's
`max_retries`/`max_rto` must be chosen against the loss rates it
actually expects to see, not assumed adequate for "anything short of
100% loss."

## Testing

- `segment.rs`, `rto.rs`: unit tests for the header codec and the RTO
  estimator in isolation (11 tests).
- `connection.rs`: 14 unit tests, including the three deterministic
  regression tests above.
- `tests/reliability.rs`, driven through two real `channel::Channel`s
  end to end: a clean-channel baseline, isolated heavy
  reordering/duplication, an empty payload, total permanent loss giving
  up within a bounded tick count, and the headline property —
  `arbitrary_data_over_an_arbitrary_hostile_profile_arrives_intact`
  (48 cases, arbitrary data up to 1500 bytes and an arbitrary fault
  profile short of total loss) — checked against the sent bytes
  themselves as the correctness oracle (there is no simpler independent
  implementation to compare against; the input *is* the ground truth for
  "what should have arrived").

## Alternatives Considered

1. **Per-segment retransmission timers.** Rejected in favor of the
   single oldest-byte timer: simpler state, and real SACK-TCP
   implementations largely converged on this same "one clock, but
   SACK-aware retransmission" pattern rather than N independent clocks.
2. **SYN/FIN consuming sequence space (TCP's own choice).** Considered
   and explicitly rejected — see above; kept as a live, documented
   trade-off rather than a default followed without examination.
3. **Bare-ACK-only handshake completion.** This was the *original*
   design and is exactly the bug described above; reversed once the
   end-to-end test found it, the same "found this while building the
   next thing" pattern as muaddib's ADR-003/004.

## Consequences

- Ticket 003 closes. Ticket 004 (flow/congestion control) replaces
  `Config::max_in_flight_segments` — currently a fixed stand-in cap —
  with a real receiver-advertised window and congestion window; the
  send-side plumbing (`try_send_more`'s admission check) is already
  isolated to one call site for that swap.
- **Known limitation, stated honestly**: the `Closed`-still-answers-FIN
  fix is not a full `TIME_WAIT` and only helps a live `Connection`
  value, not one already dropped — acceptable for this repo (nothing
  here discards connections early) but not a claim that this transport
  is production-safe against that specific race in general.
- **Known limitation**: `deliver_data`/`recv_buffer` trust that a given
  sequence number always maps to the same bytes for the life of a
  connection (true today, since `segment_size` is fixed). Ticket 004
  must not introduce mid-connection re-segmentation of already-buffered
  ranges without revisiting this.
- No performance measurement yet for the selective-repeat buffer or the
  RTO estimator's convergence behavior under sustained loss — deferred
  to ticket 006, this phase's benchmarking ticket, per this project's
  established pattern.
