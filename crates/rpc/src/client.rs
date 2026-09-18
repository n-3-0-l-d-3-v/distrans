//! The RPC client: assigns request ids, retries a call that hasn't been
//! answered within a deadline, and hands completed responses back to the
//! caller. Poll-driven, matching `transport::Connection`'s own style —
//! `RpcClient` owns one `Connection` and every method returns the raw
//! datagrams that now need to go out.
//!
//! **Why retry at all, when `Connection` already retries reliably?**
//! `Connection`'s retries defend against packet loss on an otherwise
//! healthy connection; they say nothing about a connection that resets,
//! or a caller unwilling to wait as long as `Connection`'s own retry
//! budget might take. `RpcClient`'s retry is a second, independent layer
//! of patience — and because it can genuinely re-deliver the *same*
//! logical request while `Connection`'s own retry for the first attempt
//! is still in flight, the server-side dedup table (`server.rs`) is not
//! a defensive nicety, it is load-bearing: without it, a client retry
//! would make a request's handler run twice.

use std::collections::HashMap;

use transport::{Connection, Tick};

use crate::framing::{frame, FrameReader};
use crate::message::{Request, Response};

struct PendingCall {
    request: Request,
    sent_at: Tick,
    attempts: u32,
}

pub struct RpcClient {
    conn: Connection,
    reader: FrameReader,
    next_id: u64,
    pending: HashMap<u64, PendingCall>,
    completed: Vec<(u64, Response)>,
    /// Ticks to wait for a response before resending the same request.
    pub retry_deadline: u64,
    /// A request retried this many times without an answer fails the
    /// call (returned via `poll_failed`).
    pub max_attempts: u32,
    failed: Vec<u64>,
}

impl RpcClient {
    pub fn new(conn: Connection, retry_deadline: u64, max_attempts: u32) -> Self {
        Self {
            conn,
            reader: FrameReader::new(),
            next_id: 0,
            pending: HashMap::new(),
            completed: Vec::new(),
            retry_deadline,
            max_attempts,
            failed: Vec::new(),
        }
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Issues a new call, returning its request id (to correlate with a
    /// later `poll_completed`/`poll_failed`) and the datagrams to send.
    pub fn call(&mut self, now: Tick, method: u8, payload: &[u8]) -> (u64, Vec<Vec<u8>>) {
        let id = self.next_id;
        self.next_id += 1;
        let request = Request {
            id,
            method,
            payload: payload.to_vec(),
        };
        let out = self.conn.send(now, &frame(&request.encode()));
        self.pending.insert(
            id,
            PendingCall {
                request,
                sent_at: now,
                attempts: 1,
            },
        );
        (id, out)
    }

    pub fn on_datagram(&mut self, now: Tick, datagram: &[u8]) -> Vec<Vec<u8>> {
        let out = self.conn.on_datagram(now, datagram);
        self.reader.feed(&self.conn.recv());
        for bytes in self.reader.drain_frames() {
            if let Some(response) = Response::decode(&bytes) {
                if self.pending.remove(&response.id).is_some() {
                    self.completed.push((response.id, response));
                }
                // A response for an id we don't recognize (already
                // completed, or never ours) is silently ignored — could
                // be a duplicate response to a request we retried, whose
                // first answer already completed the call.
            }
        }
        out
    }

    /// Resends any call that's been waiting past `retry_deadline`, and
    /// forwards the underlying connection's own timer.
    pub fn on_tick(&mut self, now: Tick) -> Vec<Vec<u8>> {
        let mut out = self.conn.on_tick(now);
        let mut to_fail = Vec::new();
        for (&id, call) in self.pending.iter_mut() {
            if (now - call.sent_at) >= self.retry_deadline {
                call.attempts += 1;
                if call.attempts > self.max_attempts {
                    to_fail.push(id);
                    continue;
                }
                call.sent_at = now;
                out.extend(self.conn.send(now, &frame(&call.request.encode())));
            }
        }
        for id in to_fail {
            self.pending.remove(&id);
            self.failed.push(id);
        }
        out
    }

    /// Every call that has received its answer since the last poll.
    pub fn poll_completed(&mut self) -> Vec<(u64, Response)> {
        std::mem::take(&mut self.completed)
    }

    /// Every call that exceeded `max_attempts` without an answer.
    pub fn poll_failed(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.failed)
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}
