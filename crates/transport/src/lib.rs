//! Reliable, ordered, exactly-once-delivered byte transport over the
//! hostile channel (ticket 003), per `docs/design/WIRE.md`.

mod connection;
mod rto;
mod segment;

pub use connection::{Config, Connection, FailureReason, State, Stats, Tick};
pub use segment::{SackRange, SegmentHeader};
