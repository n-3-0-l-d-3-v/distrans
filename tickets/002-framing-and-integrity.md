---
status: open
phase: 5
---

# 002 — Framing and integrity

Turn a raw byte payload into a self-describing, corruption-detecting unit
that can be sent over `channel`'s raw datagrams.

## Scope
- A versioned binary frame header (version, type, sequence number,
  payload length, flags) plus a CRC-32C trailer over header+payload.
- `encode`/`decode`, with `decode` returning a typed error (never
  panicking) for: truncated header, truncated payload, length mismatch,
  unknown version, and CRC mismatch — each a distinct variant.
- Property test: `decode(encode(f)) == Ok(f)` for arbitrary frames.
- Property test: an arbitrary single-, double- or odd-count bit flip
  anywhere in an encoded frame is always caught by the CRC (this is a
  documented property of CRC-32C for frame sizes in this repo's range —
  proved here empirically for a large sample, not just asserted from the
  literature).
- Property test: decoding an arbitrary truncated prefix of an encoded
  frame never panics and never returns `Ok` with a value different from
  what a correct decode of a non-truncated frame would give.
- Fed through `channel`'s corruption/truncation faults directly (a small
  integration test), to prove ticket 001 and ticket 002 actually compose.

Not started. Depends on ticket 001.
