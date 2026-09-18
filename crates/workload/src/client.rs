//! Thin convenience for issuing `kv::Op`s through an `rpc::RpcClient`.

use rpc::RpcClient;
use transport::Tick;

use crate::kv::Op;

pub fn call(client: &mut RpcClient, now: Tick, op: &Op) -> (u64, Vec<Vec<u8>>) {
    client.call(now, op.method(), &op.encode_payload())
}
