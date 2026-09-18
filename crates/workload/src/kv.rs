//! The closing workload's real service: a key-value store (`get`/`put`/
//! `delete`) served over `rpc`. Protocol encoding lives here; the actual
//! storage (`ReferenceStore`) is deliberately a second, independent
//! implementation of the same three operations, used only to check the
//! real server's behavior differentially in `chaos.rs` — not the type
//! the real server uses internally (that's a plain `HashMap` closed over
//! by the handler in `server.rs`).

use std::collections::HashMap;

pub const METHOD_GET: u8 = 0;
pub const METHOD_PUT: u8 = 1;
pub const METHOD_DELETE: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Get { key: Vec<u8> },
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

impl Op {
    pub fn method(&self) -> u8 {
        match self {
            Op::Get { .. } => METHOD_GET,
            Op::Put { .. } => METHOD_PUT,
            Op::Delete { .. } => METHOD_DELETE,
        }
    }

    /// Encodes this op's request payload (the method itself travels in
    /// `rpc::Request::method`, not here).
    pub fn encode_payload(&self) -> Vec<u8> {
        match self {
            Op::Get { key } | Op::Delete { key } => key.clone(),
            Op::Put { key, value } => {
                let mut out = Vec::with_capacity(2 + key.len() + value.len());
                out.extend_from_slice(&(key.len() as u16).to_le_bytes());
                out.extend_from_slice(key);
                out.extend_from_slice(value);
                out
            }
        }
    }

    pub fn decode(method: u8, payload: &[u8]) -> Option<Op> {
        match method {
            METHOD_GET => Some(Op::Get {
                key: payload.to_vec(),
            }),
            METHOD_DELETE => Some(Op::Delete {
                key: payload.to_vec(),
            }),
            METHOD_PUT => {
                if payload.len() < 2 {
                    return None;
                }
                let key_len = u16::from_le_bytes(payload[0..2].try_into().unwrap()) as usize;
                if payload.len() < 2 + key_len {
                    return None;
                }
                Some(Op::Put {
                    key: payload[2..2 + key_len].to_vec(),
                    value: payload[2 + key_len..].to_vec(),
                })
            }
            _ => None,
        }
    }
}

/// What executing an `Op` against a store produces: `(ok, payload)`,
/// matching `rpc`'s own `(bool, Vec<u8>)` handler signature exactly.
/// `Get`/`Delete` report `ok=false` for a missing key (not an error —
/// just "not found"); `Put` always succeeds.
pub fn apply(store: &mut HashMap<Vec<u8>, Vec<u8>>, op: &Op) -> (bool, Vec<u8>) {
    match op {
        Op::Get { key } => match store.get(key) {
            Some(v) => (true, v.clone()),
            None => (false, Vec::new()),
        },
        Op::Put { key, value } => {
            store.insert(key.clone(), value.clone());
            (true, Vec::new())
        }
        Op::Delete { key } => {
            let existed = store.remove(key).is_some();
            (existed, Vec::new())
        }
    }
}

/// A second, independently written implementation of the exact same
/// three operations, used only as a differential oracle in
/// `chaos.rs` — deliberately not shared code with `apply` above, so a
/// bug in one is very unlikely to also be in the other.
#[derive(Debug, Default)]
pub struct ReferenceStore {
    entries: Vec<(Vec<u8>, Vec<u8>)>,
}

impl ReferenceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, op: &Op) -> (bool, Vec<u8>) {
        match op {
            Op::Get { key } => {
                for (k, v) in self.entries.iter().rev() {
                    if k == key {
                        return (true, v.clone());
                    }
                }
                (false, Vec::new())
            }
            Op::Put { key, value } => {
                self.entries.retain(|(k, _)| k != key);
                self.entries.push((key.clone(), value.clone()));
                (true, Vec::new())
            }
            Op::Delete { key } => {
                let before = self.entries.len();
                self.entries.retain(|(k, _)| k != key);
                (self.entries.len() != before, Vec::new())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_encoding_round_trips() {
        for op in [
            Op::Get { key: b"k".to_vec() },
            Op::Delete {
                key: b"k2".to_vec(),
            },
            Op::Put {
                key: b"k3".to_vec(),
                value: b"v".to_vec(),
            },
        ] {
            let decoded = Op::decode(op.method(), &op.encode_payload()).unwrap();
            assert_eq!(decoded, op);
        }
    }

    #[test]
    fn apply_and_reference_store_agree_on_a_fixed_sequence() {
        let mut store = HashMap::new();
        let mut reference = ReferenceStore::new();
        let ops = [
            Op::Put {
                key: b"a".to_vec(),
                value: b"1".to_vec(),
            },
            Op::Get { key: b"a".to_vec() },
            Op::Get {
                key: b"missing".to_vec(),
            },
            Op::Delete { key: b"a".to_vec() },
            Op::Get { key: b"a".to_vec() },
            Op::Delete { key: b"a".to_vec() },
            Op::Put {
                key: b"a".to_vec(),
                value: b"2".to_vec(),
            },
            Op::Put {
                key: b"a".to_vec(),
                value: b"3".to_vec(),
            },
            Op::Get { key: b"a".to_vec() },
        ];
        for op in &ops {
            assert_eq!(apply(&mut store, op), reference.apply(op), "{op:?}");
        }
    }
}
