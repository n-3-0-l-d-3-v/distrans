//! Request/response messaging over `transport`'s reliable byte stream
//! (ticket 005), with the one property a reliable stream doesn't give
//! for free: a client that retries a request it couldn't confirm was
//! answered must never cause that request's handler to run twice.
//!
//! `transport::Connection` is stream-oriented (byte offsets, no message
//! boundaries), so `framing` reassembles length-prefixed frames from it
//! before anything here looks at request/response structure.
//!
//! See `docs/design/decisions/ADR-005-rpc.md`.

mod client;
mod framing;
mod message;
mod server;

pub use client::RpcClient;
pub use framing::{frame, FrameReader};
pub use message::{Request, Response};
pub use server::{Handler, RpcServer};
