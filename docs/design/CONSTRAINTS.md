# Constraints — THE WIRE

## Primary constraint

The underlying channel provides no ordering, no reliable delivery, no framing, no integrity, and duplicates or corrupts datagrams. All of that must be built at this layer.

## What it forces

Framing, checksums/integrity, sequence numbers, ACKs, retransmission, selective repeat, flow control, and RPC with idempotency.

## Research question

How much of TCP's design is forced by physics versus by historical convention?

## What is explicitly out of scope

See the root [SCOPE.md](../../SCOPE.md) for the CORE / EXTENSION / EXPERIMENT
classification that applies to this repo.
