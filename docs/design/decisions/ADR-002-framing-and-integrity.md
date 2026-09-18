# ADR-002: A fixed 12-byte header, CRC-32C over header+payload, ordered fail-fast decoding

## Status
Accepted

## Context

Ticket 002 needs a wire format that a transport layer can build on: a
way to tell where one datagram's meaningful content starts and ends, and
a way to know whether the bytes that arrived are the bytes that were
sent, given `channel`'s hostility (corruption, truncation, and — via
`frame` sitting *above* `channel` — arbitrary garbage from any source).
`decode` is this whole repo's trust boundary: every later layer only
sees a `Frame` decode has already validated.

## Decision

**Fixed 12-byte header, little-endian, then payload, then a 4-byte
CRC-32C trailer over everything preceding it.** Fixed-width fields (no
variable-length encoding) keep `decode` simple and panic-free: every
fixed-offset read is bounds-checked once, up front, by the length check.
Fields: version (1 byte), frame type (1 byte, uninterpreted — later
layers give it meaning), sequence number (u32), flags (u16), payload
length (u32).

**CRC-32C (Castagnoli), hand-rolled bit-at-a-time.** Same reasoning as
`channel`'s `SplitMix64`: a from-scratch implementation has no external
crate's bugs to inherit, which matters specifically for the one function
in this repo whose entire job is catching corruption. CRC-32C is chosen
over the classic CRC-32 (0x04C11DB7) for its documented superior
error-detection properties at these frame sizes (Koopman 2002) — verified
here, not just cited: `every_single_bit_flip_changes_the_crc` checks
every single-bit flip of a real message, and
`odd_weight_bit_flips_are_always_caught` extends this to the CRC's
guaranteed odd-weight detection.

**Checks run cheapest-and-most-likely-to-fire first: length, then
version, then the CRC scan.** A garbage or truncated datagram (the most
common hostile-channel outcome by far) is rejected by an O(1) length
comparison, never paying for an O(n) checksum pass. The CRC covers
*version and length too* (they're part of the header, all included in
the checksum), so a corrupted length field either fails the plain
arithmetic length check immediately, or — if corruption happened to
produce internally-consistent lengths — still fails the checksum.

**Every rejection has its own error variant** (`TooShort`,
`UnsupportedVersion`, `LengthMismatch`, `ChecksumMismatch`), not a single
opaque `DecodeError`. `docs/DEFINITION_OF_DONE.md` item 3 (failure
behavior, not just happy path) applies to *diagnosing* failure too:
ticket 003's retransmission logic will care whether a datagram merely
never fully arrived (`LengthMismatch`/`TooShort`, from truncation) or
arrived complete but corrupted (`ChecksumMismatch`) — different failure
modes different logging or backoff heuristics might eventually want to
tell apart, mirroring muaddib's `Revoked`-vs-`UnknownObject` distinction
(ADR-001 there).

## Testing

- Round-trip: `decode(encode(f)) == Ok(f)` for 1024 arbitrary frames
  (`round_trips_for_arbitrary_frames`), plus explicit edge cases (empty
  payload, max-value fields).
- Bit-flip detection: every single-bit flip of a fixed message is caught
  (exhaustive unit test), and for 1024 arbitrary frames, a flip at an
  arbitrary bit position never silently produces a *different* valid
  frame (`a_single_bit_flip_anywhere_is_always_caught`) — it may only
  ever legitimately decode back to the original, or be rejected. The
  same is checked for arbitrary odd-count multi-bit flips
  (`odd_weight_bit_flips_are_always_caught`), which is CRC-32C's
  documented guarantee, not merely single-bit's.
- No panics: `decode` never panics on an arbitrary truncated prefix
  (`decoding_a_truncated_prefix_never_panics`) or arbitrary random bytes
  of arbitrary length (`decoding_arbitrary_garbage_never_panics`) — 1024
  cases each.
- **Composition, not just isolation**: `channel_integration.rs` sends
  frames through a real `Channel` with 40% corruption and 30% truncation
  probability, across 40 seeds, and requires every delivered datagram to
  either decode back to exactly the frame that was sent or be rejected
  with one of the expected error variants — proving ticket 001 and
  ticket 002 actually compose, not just that each independently passes
  its own tests.
- Mutation-checked: disabling the CRC comparison (`if false` instead of
  `if computed_crc != claimed_crc`) is caught immediately by the
  bit-flip-detection unit tests.

## Alternatives Considered

1. **Variable-length header (e.g. varint-encoded length).** Rejected:
   more decode-time branching and bounds-checking for a repo whose
   frame sizes don't need the space savings; fixed-width keeps `decode`
   trivially auditable.
2. **CRC-32 (classic, not Castagnoli).** Rejected in favor of CRC-32C's
   better-documented burst/odd-weight detection at small frame sizes,
   and because CRC-32C is what real protocols in this exact problem
   space (iSCSI, SCTP) chose for the same reason.
3. **A single opaque `DecodeError` string.** Rejected: loses information
   ticket 003 will want (see above), and this project's convention
   (muaddib's `CapError`, `IpcError`, `MemoryError`) is always
   distinguishable, matchable error variants.

## Consequences

- Ticket 002 closes. Ticket 003 (transport) can build directly on
  `Frame`'s `sequence`/`flags`/`frame_type` fields — no changes needed to
  this format for sequencing, ACK flags, or distinguishing data frames
  from control frames.
- **Known limitation, deferred honestly**: CRC-32C detects corruption
  but doesn't defend against a deliberately adversarial attacker who
  can compute valid checksums (this repo's channel corrupts randomly or
  via explicit test scripts, never intelligently) — no cryptographic
  integrity (HMAC/AEAD) is in scope for this phase's research question,
  which is about reliability over an unreliable channel, not security
  over a hostile one in the adversarial-attacker sense.
- Header overhead is a fixed 16 bytes (12 header + 4 CRC) per frame,
  regardless of payload size — not yet measured against payload size for
  small messages; deferred to ticket 006 alongside this phase's other
  performance work.
