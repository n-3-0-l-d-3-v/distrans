//! Ticket 006: THE WIRE's closing workload.
//!
//! - `kv`: the real service (get/put/delete) and its protocol encoding,
//!   plus an independently-written `ReferenceStore` used only as a
//!   differential oracle in `chaos`.
//! - `server`: wires the kv service onto `rpc::RpcServer`; `client` onto
//!   `rpc::RpcClient` for driving it.
//! - `baselines`: two textbook ARQ schemes (stop-and-wait, go-back-N)
//!   built directly on `frame`/`channel`, independent of `transport`, for
//!   a fair goodput comparison.
//! - `chaos`: a seeded multi-client chaos harness checked against `kv`'s
//!   `ReferenceStore`.
//! - `profiles`: the named hostile-channel profiles this ticket drives
//!   the workload through.
//!
//! See `docs/design/decisions/ADR-006-integration-and-benchmarks.md`.

pub mod baselines;
pub mod chaos;
pub mod client;
pub mod kv;
pub mod profiles;
pub mod rng;
pub mod server;
