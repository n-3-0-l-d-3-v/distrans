---
status: done
phase: 5
---

# 002 — Framing and integrity

Turn a raw byte payload into a self-describing, corruption-detecting unit
that can be sent over `channel`'s raw datagrams.

## Scope
- [x] A versioned binary frame header (version, type, sequence number,
      payload length, flags) plus a CRC-32C trailer over header+payload.
- [x] `encode`/`decode`, with `decode` returning a typed error (never
      panicking) for: truncated header, truncated payload, length
      mismatch, unknown version, and CRC mismatch — each a distinct
      variant.
- [x] Property test: `decode(encode(f)) == Ok(f)` for arbitrary frames
      (1024 cases).
- [x] Property test: an arbitrary single-bit or odd-count bit flip
      anywhere in an encoded frame is always caught by the CRC, or (if it
      lands where it doesn't affect validity) decodes back to the
      original — never a silently different valid frame.
- [x] Property test: decoding an arbitrary truncated prefix, or arbitrary
      random garbage, of any length, never panics and never returns `Ok`
      with a value different from the original frame.
- [x] Fed through `channel`'s corruption/truncation faults directly
      (`channel_integration.rs`), proving tickets 001 and 002 compose.

15 unit tests, 5 property tests (1024 cases each), 3 channel-integration
tests. Mutation-checked (disabling the CRC comparison is caught
immediately). See
`docs/design/decisions/ADR-002-framing-and-integrity.md`.
