//! Request/response message encoding, carried inside `framing`'s
//! length-prefixed frames.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub method: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub id: u64,
    /// Whether the handler completed successfully — a real error path
    /// (e.g. "key not found" for ticket 006's key-value service), not
    /// just success/payload.
    pub ok: bool,
    pub payload: Vec<u8>,
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(9 + self.payload.len());
        out.extend_from_slice(&self.id.to_le_bytes());
        out.push(self.method);
        out.extend_from_slice(&self.payload);
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Request> {
        if bytes.len() < 9 {
            return None;
        }
        Some(Request {
            id: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            method: bytes[8],
            payload: bytes[9..].to_vec(),
        })
    }
}

impl Response {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(9 + self.payload.len());
        out.extend_from_slice(&self.id.to_le_bytes());
        out.push(u8::from(self.ok));
        out.extend_from_slice(&self.payload);
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Response> {
        if bytes.len() < 9 {
            return None;
        }
        Some(Response {
            id: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            ok: bytes[8] != 0,
            payload: bytes[9..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips() {
        let r = Request {
            id: 42,
            method: 3,
            payload: b"hello".to_vec(),
        };
        assert_eq!(Request::decode(&r.encode()), Some(r));
    }

    #[test]
    fn response_round_trips_ok_and_err() {
        for ok in [true, false] {
            let r = Response {
                id: 7,
                ok,
                payload: b"result".to_vec(),
            };
            assert_eq!(Response::decode(&r.encode()), Some(r));
        }
    }

    #[test]
    fn decode_rejects_too_short_input() {
        assert_eq!(Request::decode(&[1, 2, 3]), None);
        assert_eq!(Response::decode(&[1, 2, 3]), None);
    }

    #[test]
    fn empty_payload_round_trips() {
        let r = Request {
            id: 0,
            method: 0,
            payload: vec![],
        };
        assert_eq!(Request::decode(&r.encode()), Some(r));
    }
}
