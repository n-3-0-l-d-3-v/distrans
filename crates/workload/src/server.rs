//! Wires `kv`'s get/put/delete operations onto an `rpc::RpcServer`. The
//! store is a plain `HashMap` shared (via `Rc<RefCell<_>>`) across every
//! client connection's own `RpcServer` instance — this crate's
//! simulation is single-threaded, so `Rc<RefCell<_>>` is the right tool,
//! not a stand-in for a real concurrent store.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rpc::{Handler, RpcServer};
use transport::Connection;

use crate::kv::{self, Op};

pub type SharedStore = Rc<RefCell<HashMap<Vec<u8>, Vec<u8>>>>;

/// Builds the handler one `RpcServer` (one accepted connection) uses,
/// closing over the store shared with every other connection.
/// `on_execute` is called once per actual (non-duplicate) execution, in
/// the exact order the server processes requests — `chaos.rs` uses it to
/// build its differential replay log.
pub fn make_server<'a>(
    conn: Connection,
    store: SharedStore,
    mut on_execute: impl FnMut(&Op, bool, &[u8]) + 'a,
) -> RpcServer<'a> {
    let handler: Handler<'a> = Box::new(move |method, payload| {
        let Some(op) = Op::decode(method, payload) else {
            return (false, Vec::new());
        };
        let (ok, response) = kv::apply(&mut store.borrow_mut(), &op);
        on_execute(&op, ok, &response);
        (ok, response)
    });
    RpcServer::new(conn, handler)
}
