//! The RPC server side of one connection: decodes requests, dispatches
//! to a handler, and guarantees each request id's handler runs **at most
//! once** — a retried or duplicated request gets the cached response
//! replayed, never a second execution. One `RpcServer` serves one
//! `Connection`; a listener accepting several clients (ticket 006's
//! workload) runs one `RpcServer` per accepted connection, sharing
//! whatever state the handler closes over.

use std::collections::HashMap;

use transport::{Connection, Tick};

use crate::framing::{frame, FrameReader};
use crate::message::{Request, Response};

/// A request handler: method id and payload in, `(ok, response payload)`
/// out. `FnMut` so it can close over shared, mutable application state
/// (a key-value store, a counter, ...) — exactly the kind of observable
/// side effect ticket 005's property test needs to prove happens at most
/// once per request id.
pub type Handler<'a> = Box<dyn FnMut(u8, &[u8]) -> (bool, Vec<u8>) + 'a>;

pub struct RpcServer<'a> {
    conn: Connection,
    reader: FrameReader,
    handler: Handler<'a>,
    /// Every request id ever seen, with the response its (single) real
    /// execution produced. Never evicted in this ticket's scope — an
    /// unbounded dedup table is a known, stated limitation (see
    /// ADR-005); a real server would need to expire entries once a
    /// client acknowledges it will never retry that id again.
    seen: HashMap<u64, Response>,
    executions: u64,
}

impl<'a> RpcServer<'a> {
    pub fn new(conn: Connection, handler: Handler<'a>) -> Self {
        Self {
            conn,
            reader: FrameReader::new(),
            handler,
            seen: HashMap::new(),
            executions: 0,
        }
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// How many times the handler has actually run — the number a
    /// correct implementation keeps equal to the number of *distinct*
    /// request ids ever received, no matter how many times each was
    /// retried.
    pub fn executions(&self) -> u64 {
        self.executions
    }

    pub fn on_datagram(&mut self, now: Tick, datagram: &[u8]) -> Vec<Vec<u8>> {
        let mut out = self.conn.on_datagram(now, datagram);
        self.reader.feed(&self.conn.recv());
        for bytes in self.reader.drain_frames() {
            let Some(request) = Request::decode(&bytes) else {
                continue; // malformed request frame: drop, like a corrupted datagram upstream
            };
            let response = if let Some(cached) = self.seen.get(&request.id) {
                cached.clone()
            } else {
                let (ok, payload) = (self.handler)(request.method, &request.payload);
                self.executions += 1;
                let response = Response {
                    id: request.id,
                    ok,
                    payload,
                };
                self.seen.insert(request.id, response.clone());
                response
            };
            out.extend(self.conn.send(now, &frame(&response.encode())));
        }
        out
    }

    pub fn on_tick(&mut self, now: Tick) -> Vec<Vec<u8>> {
        self.conn.on_tick(now)
    }
}
