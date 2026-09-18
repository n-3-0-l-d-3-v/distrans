//! Ticket 005's headline guarantee, proven end to end over real
//! `channel::Channel`s and a real `transport::Connection` pair: for an
//! arbitrary sequence of client calls, arbitrary fault profiles, and a
//! client retry deadline short enough to guarantee spurious retries even
//! on a healthy connection, every request's handler executes **at most
//! once**, and every response the client ever sees for a given request
//! matches what that one execution produced.

use std::cell::RefCell;
use std::rc::Rc;

use channel::{Channel, FaultProfile, Tick};
use proptest::prelude::*;
use rpc::{RpcClient, RpcServer};
use transport::{Config, Connection};

fn transport_config() -> Config {
    Config {
        segment_size: 64,
        recv_window_capacity: 8 * 64,
        min_rto: 2,
        max_rto: 300,
        max_retries: 30,
        fast_retransmit_dup_acks: 3,
    }
}

/// Drives an `RpcClient`/`RpcServer` pair over two hostile `Channel`s
/// until every issued call has either completed or failed, or
/// `max_ticks` elapses.
struct Sim {
    client: RpcClient,
    server: RpcServer<'static>,
    c2s: Channel,
    s2c: Channel,
    tick: u64,
}

impl Sim {
    fn new(
        seed_c2s: u64,
        seed_s2c: u64,
        profile: FaultProfile,
        retry_deadline: u64,
        executions: Rc<RefCell<u64>>,
    ) -> (Self, Vec<u8>) {
        let (client_conn, syn) = Connection::connect(transport_config(), Tick(0));
        let server_conn = Connection::listen(transport_config(), Tick(0));
        let client = RpcClient::new(client_conn, retry_deadline, 50);
        // The handler's side effect (an incrementing counter, folded into
        // the response payload) is what makes a re-execution observably
        // different from a replayed cached response — the property below
        // checks exactly that every response for a given request id is
        // byte-identical across however many times it was retried.
        let handler: rpc::Handler<'static> = Box::new(move |method, payload| {
            *executions.borrow_mut() += 1;
            let mut out = vec![method];
            out.extend_from_slice(payload);
            out.push(*executions.borrow() as u8);
            (true, out)
        });
        let server = RpcServer::new(server_conn, handler);
        (
            Self {
                client,
                server,
                c2s: Channel::new(seed_c2s, profile),
                s2c: Channel::new(seed_s2c, profile),
                tick: 0,
            },
            syn,
        )
    }

    fn step(&mut self) {
        self.tick += 1;
        let now = Tick(self.tick);
        for d in self.c2s.advance(now) {
            for out in self.server.on_datagram(now, &d) {
                self.s2c.send(&out);
            }
        }
        for d in self.s2c.advance(now) {
            for out in self.client.on_datagram(now, &d) {
                self.c2s.send(&out);
            }
        }
        for out in self.client.on_tick(now) {
            self.c2s.send(&out);
        }
        for out in self.server.on_tick(now) {
            self.s2c.send(&out);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn every_requests_handler_runs_at_most_once_under_retries_and_faults(
        seed in any::<u64>(),
        loss in 0.0f64..0.2,
        corruption in 0.0f64..0.1,
        reorder in 0u64..6,
        // A short, fixed retry deadline: guarantees the client retries
        // *often*, independent of whether the channel actually lost
        // anything — the harshest, most realistic test of idempotency,
        // since it's exactly what a real impatient client does.
        retry_deadline in 3u64..15,
        payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..40), 1..12),
    ) {
        let profile = FaultProfile {
            loss,
            duplication: 0.05,
            reorder_max_delay: reorder,
            corruption,
            truncation: 0.02,
            base_delay: 1,
        };
        let executions = Rc::new(RefCell::new(0u64));
        let (mut sim, syn) = Sim::new(seed, seed.wrapping_add(1), profile, retry_deadline, executions.clone());
        sim.c2s.send(&syn);

        let mut expected_responses: std::collections::HashMap<u64, Option<Vec<u8>>> = std::collections::HashMap::new();
        let mut all_ids = Vec::new();

        // Issue every call up front (all at tick ~1, once the connection
        // will soon be established) — the harness below waits for
        // Established before actually sending them.
        let mut to_issue: Vec<Vec<u8>> = payloads;
        let mut issued = false;

        let max_ticks = 30_000u64;
        loop {
            sim.step();
            let now = Tick(sim.tick);

            if !issued && matches!(sim.client.connection().state(), transport::State::Established) {
                for payload in to_issue.drain(..) {
                    let (id, out) = sim.client.call(now, 7, &payload);
                    for d in out {
                        sim.c2s.send(&d);
                    }
                    expected_responses.insert(id, None);
                    all_ids.push(id);
                }
                issued = true;
            }

            for (id, response) in sim.client.poll_completed() {
                let slot = expected_responses.get_mut(&id).expect("only ever polling ids we issued");
                match slot {
                    None => *slot = Some(response.payload),
                    Some(first) => {
                        prop_assert_eq!(
                            &response.payload, first,
                            "request {} got two different response payloads across retries -- handler ran more than once", id
                        );
                    }
                }
            }
            let failed = sim.client.poll_failed();
            prop_assert!(failed.is_empty(), "a call failed outright: {failed:?}");

            let all_done = issued && expected_responses.values().all(|v| v.is_some());
            if all_done || sim.tick >= max_ticks {
                prop_assert!(all_done, "not all calls completed within the tick budget");
                break;
            }
        }

        prop_assert_eq!(
            *executions.borrow(),
            all_ids.len() as u64,
            "handler executions ({}) must equal the number of distinct requests issued ({})",
            executions.borrow(),
            all_ids.len()
        );
        prop_assert_eq!(sim.server.executions(), all_ids.len() as u64);
    }
}
