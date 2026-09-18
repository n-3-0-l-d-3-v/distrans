# ADR-001: Virtual-tick time and seeded, at-send-time fault injection

## Status
Accepted

## Context

Everything in this repo runs over `docs/design/CONSTRAINTS.md`'s hostile
channel: no ordering, no reliable delivery, no framing, no integrity,
duplication and corruption possible. Per the Definition of Done, any
failure found later (a transport bug that only shows up under one
specific loss/reorder interleaving) must be reproducible from a recorded
seed. That requirement shapes the whole design: nothing about the
channel's behavior may depend on anything except its explicit seed and
configuration.

## Decision

**Time is a `Tick` counter, advanced only by `Channel::advance`.** No
physical clock is read anywhere (mirrors muaddib's `no_wall_clock`
pattern; a `no_wall_clock` guard test is added here too, covering
`channel`'s own `src/` and left ready for later crates to extend). A
`Channel` never advances itself — a caller drives the whole simulation's
notion of time forward explicitly, which is what makes a run replayable:
the same caller code plus the same seed produces the same tick sequence.

**Faults are decided at `send` time, not at delivery time.** Loss,
duplication, corruption, and truncation are properties of the datagram
being sent; only the *extra reordering delay* affects when it's
deliverable. This ordering matters for reproducibility: the RNG is
consumed in a fixed sequence per `send` call regardless of how many
other datagrams happen to be in flight or when `advance` is called, so
replay doesn't depend on the caller's polling pattern.

**A hand-rolled `SplitMix64`, not `rand`.** Same reasoning as muaddib's
`workload::rng::SplitMix64`: an external crate's PRNG stream isn't
guaranteed stable across its own version bumps, which would silently
break `(seed, profile)` replay after an unrelated `cargo update`.

**`Script` is a separate, additive override mechanism, not a replacement
for `FaultProfile`.** A scripted rule for a given fault (e.g.
`ScriptedFault::Drop` for datagram index 7) forces that fault
deterministically for that one datagram; every *other* fault on that same
datagram (corruption, truncation, delay) still draws randomly from the
profile unless it also has its own rule. This lets a test pin exactly the
one behavior it's checking ("datagram 7 is dropped") while leaving the
rest of the channel's behavior under normal random control — useful for
both a targeted regression test and, later, a scripted schedule embedded
in a larger random chaos run.

**A `Channel` is one direction.** A full-duplex link is two `Channel`s.
Real hostile links are frequently asymmetric (satellite uplink vs.
downlink, NAT'd asymmetric paths), so this is a closer model than a
single bidirectional object, and it's simpler: `Channel` never has to
reason about "which direction is this fault happening in."

## Testing

- Reproducibility: `same_seed_and_profile_reproduce_exactly` proves a
  `(seed, profile)` run against the same sends is byte-for-byte and
  stats-for-stats identical, for 256 arbitrary profiles and payload
  batches.
- The RNG is genuinely consumed, not a no-op: at a mid-band probability
  and 500 sends, 16 distinct seeds are not all identical
  (`different_seeds_produce_different_outcomes_at_a_mid_band_probability`
  — deliberately not a shrinkable proptest property, because two
  arbitrary small-input seeds can coincidentally agree at extreme
  probabilities near 0 or 1, which made an earlier version of this test
  flaky under shrinking; a fixed, generously-sized direct test avoids
  that).
- Statistical accuracy: `observed_fault_rates_match_configured_probabilities`
  checks loss/duplication/corruption/truncation each land within 3
  percentage points of their configured probability at N=20,000, across
  5 probabilities and 3 seeds each.
- Mutation-checked: disabling the loss draw (`dropped = scripted_drop &&
  false`) is caught by both the rate-consistency test and the
  seed-distinctness test, as expected — a channel that never actually
  drops anything still "reproduces" and still "uses its RNG" for other
  faults, so those two tests, not the reproducibility one, are the ones
  that catch this class of bug.

## Alternatives Considered

1. **Delivery-time fault decisions** (decide loss/corruption when a
   datagram would be delivered, based on ticks elapsed). Rejected:
   couples the RNG's consumption order to the caller's `advance`
   granularity, which would make replay depend on exactly how often the
   caller polls — a subtle non-determinism this repo's whole reproducibility
   story exists to avoid.
2. **`rand::SeedableRng` (e.g. `SmallRng`).** Rejected for the same
   reason muaddib avoided it: no cross-version stream stability guarantee.
3. **A single bidirectional `Channel`.** Rejected: real hostile links are
   commonly asymmetric, and two independent `Channel`s is both a more
   accurate model and less internal branching.

## Consequences

- Ticket 001 closes. Ticket 002 (framing) can be tested by piping
  `channel`'s corruption/truncation faults directly at `frame::decode`,
  proving the two crates compose for real rather than only in isolation.
- **Known limitation**: reordering is delay-based (an extra random delay
  before the normal base delay), not an explicit "swap these two
  datagrams" fault. This can reorder adjacent sends but doesn't guarantee
  an arbitrary permutation is reachable in one channel lifetime; ticket
  003's transport tests will need enough reordering probability and
  enough sends for the selective-repeat property tests to exercise real
  out-of-order delivery, not assume it from a single configuration.
- `Channel::send` is `O(log n)` in the number of currently in-flight
  datagrams (binary heap); not benchmarked yet — deferred to ticket 006
  alongside the rest of this phase's performance work, per this
  project's established pattern of measuring rather than assuming.
