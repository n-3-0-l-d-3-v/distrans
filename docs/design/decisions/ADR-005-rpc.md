# ADR-005: Length-prefixed framing over a byte stream, a second independent retry layer, and why the dedup table is load-bearing

## Status
Accepted

## Context

Ticket 003 gave this repo a reliable, ordered byte *stream*. RPC needs
discrete request/response *messages*, and — the property this ticket
actually exists to prove — a client that can't tell whether its request
or its response was the thing that went missing must be able to retry
without ever causing the request's handler to run twice.

## Decision

**`transport::Connection` has no message boundaries; `rpc::framing`
reassembles them.** `Connection::send`/`recv` operate on raw bytes,
segmented into transport frames however `segment_size` and admission
happen to chunk them — a single `recv()` call can return a partial
message, several messages concatenated, or anything between. A
length-prefixed frame (4-byte LE length, then payload) on top, with a
small reassembly buffer (`FrameReader`) that only ever yields whole
frames, is the standard, minimal answer. This is a real, deliberate
consequence of ADR-003's choice to model `transport` as a byte stream
(TCP-like) rather than a message-oriented layer — a message-oriented
transport would have made this ticket's framing step unnecessary, at
the cost of building a different kind of transport in ticket 003. Not
revisited: the byte-stream model is what ticket 003 committed to and
tested thoroughly, and re-litigating it here wasn't worth reopening.

**`RpcClient` retries independently of `Connection`'s own reliable
delivery, on purpose.** This needed to be reasoned through explicitly,
because at first glance it looks redundant: `Connection` already retries
lost segments until they arrive or the connection gives up, so why would
RPC ever need to retry the same request? Because `Connection`'s retry
budget and a caller's patience are different things — a real client may
not want to wait as long as `Connection`'s own RTO/backoff schedule
might take, and (more importantly for what this ticket can actually
simulate) a connection can reset or need re-establishing, which a
reliable byte stream does not survive. `RpcClient::retry_deadline` models
that impatience directly: if a call's `Connection`-level bytes are still
honestly in flight, being retried by `Connection` itself, and `RpcClient`
gives up waiting and resends anyway, the server can legitimately receive
*two* independent, both-eventually-delivered copies of the exact same
logical request. That is not a bug to prevent by tuning timers more
carefully — it is the scenario the whole ticket exists to make safe.

**The dedup table is not a defensive nicety layered on top of an
already-safe design — it is the thing that makes retry safe at all.**
Once the design above is understood, this follows immediately: without
`RpcServer`'s `seen: HashMap<u64, Response>`, the harness's own default
configuration (a short, fixed `retry_deadline`, deliberately shorter
than a typical round trip so retries happen *routinely*, not only under
packet loss) would make most handlers run more than once for most
requests. The dedup table keyed by request id, replaying the cached
response verbatim for a duplicate, is what turns "the client may
send this twice" into "the handler runs once."

**Request ids are a plain per-client counter, not randomized.** Same
reasoning as every other identifier in this project (mentat's
`ObjectId`, muaddib's — this repo has no adversarial-attacker threat
model in scope, so there's nothing to gain from unpredictability and a
real cost (harder-to-read test failures, an extra RNG dependency) to
avoiding a monotonic counter.

## Testing

- `framing.rs`, `message.rs`: unit tests for frame reassembly (whole,
  byte-at-a-time, concatenated, partial-trailing, empty-payload) and
  request/response codecs (9 tests).
- `tests/idempotency.rs`, end to end over real `channel::Channel`s and a
  real `Connection` pair: for an arbitrary batch of calls, an arbitrary
  sub-total fault profile, and — critically — an arbitrary *short* retry
  deadline (3–15 ticks, deliberately shorter than most round trips under
  the tested profiles) chosen specifically to force frequent retries
  independent of whether the channel actually lost anything, every
  request's handler executes **exactly once**
  (`server.executions() == number of distinct requests issued`) and
  every response the client ever receives for a given request id is
  byte-identical to the first one seen (proven observably, by having the
  handler fold a monotonic execution counter into its response payload —
  a re-execution would produce a *visibly different* answer, not merely
  an unnoticed duplicate).
- Mutation-checked: replacing the server's dedup lookup with unconditional
  re-execution fails the property immediately.

## Alternatives Considered

1. **A message-oriented transport instead of framing on top of a byte
   stream.** Rejected as a ticket-005 decision — this is really a
   ticket-003 design question, already made and tested there; redoing it
   here would mean re-testing ticket 003's guarantees under a different
   model instead of building on them.
2. **Idempotency via a client-supplied idempotency key with server-side
   expiry (real-world REST APIs' usual approach).** Rejected for scope:
   expiry needs a notion of "the client will never retry this id again,"
   which needs either time (unavailable in spirit — this project avoids
   wall-clock reasoning throughout) or an explicit client acknowledgment
   protocol neither ticket asks for. A per-connection, never-evicted
   table is simpler and correct for this connection's lifetime, with the
   growth cost stated honestly below rather than solved.
3. **Randomized request ids.** Rejected — see above.

## Consequences

- Ticket 005 closes.
- **Known limitation, stated plainly**: `RpcServer`'s dedup table is
  never evicted. For a single connection's lifetime (this ticket's and
  ticket 006's scope) that's bounded by how many distinct requests one
  client sends, which is fine; a long-lived production server would need
  either a bounded LRU with a correctness argument for why eviction can't
  reintroduce a duplicate, or an explicit "you may forget request N"
  acknowledgment from the client — neither implemented here.
- `RpcServer` serves exactly one `Connection`. Ticket 006's multi-client
  key-value service runs one `RpcServer` per accepted connection,
  sharing application state through the handler closures — a direct,
  already-supported extension, not a redesign.
- Request/response framing overhead (4-byte length prefix, 9-byte
  request/response header) is small relative to `frame`'s own per-segment
  overhead (ADR-002) and not separately measured; ticket 006's benchmarks
  cover the assembled system's throughput, not this layer's overhead in
  isolation.
