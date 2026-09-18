//! Seeded, multi-client chaos testing for the key-value workload: random
//! interleavings of get/put/delete from several simulated clients,
//! optional mid-stream fault-profile changes, checked against `kv`'s
//! independently-written `ReferenceStore` by replaying the exact order
//! the real server executed things in. Every run is a pure function of
//! its `ChaosConfig` (seed included), so any failure is replayable.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use channel::{Channel, FaultProfile};
use rpc::{RpcClient, RpcServer};
use transport::{Config as TransportConfig, Connection, Tick};

use crate::client::call;
use crate::kv::{Op, ReferenceStore};
use crate::rng::SplitMix64;
use crate::server::make_server;

#[derive(Debug, Clone)]
pub struct ChaosConfig {
    pub seed: u64,
    pub clients: usize,
    /// Number of client calls to schedule across the run.
    pub calls: usize,
    pub max_ticks: u64,
    /// Fault profile in effect from tick 0.
    pub initial_profile: FaultProfile,
    /// Additional `(tick, profile)` switches, applied in order. A switch
    /// replaces every client's channel pair with freshly (deterministically)
    /// seeded ones under the new profile — anything still literally in
    /// flight at that instant is dropped, the same way a real link
    /// failover would drop in-flight frames.
    pub profile_switches: Vec<(u64, FaultProfile)>,
    pub key_space: usize,
}

impl Default for ChaosConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            clients: 3,
            calls: 60,
            max_ticks: 60_000,
            initial_profile: FaultProfile::CLEAN,
            profile_switches: Vec::new(),
            key_space: 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChaosFailure {
    pub seed: u64,
    pub message: String,
}

impl std::fmt::Display for ChaosFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "chaos invariant violated (seed {:#x}): {}\n  replay: distrans-chaos --seed {:#x}",
            self.seed, self.message, self.seed
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChaosSummary {
    pub seed: u64,
    pub executions: usize,
    pub calls_completed: usize,
    pub calls_failed: usize,
    pub ticks_used: u64,
}

fn transport_config() -> TransportConfig {
    TransportConfig {
        segment_size: 128,
        recv_window_capacity: 8 * 128,
        min_rto: 2,
        max_rto: 400,
        max_retries: 40,
        fast_retransmit_dup_acks: 3,
    }
}

type ExecLog = Rc<RefCell<Vec<(Op, bool, Vec<u8>)>>>;

fn make_link(seed: u64, profile: FaultProfile) -> (Channel, Channel) {
    (
        Channel::new(seed, profile),
        Channel::new(seed ^ 0xABCD_EF01_2345_6789, profile),
    )
}

struct ClientLink {
    client: RpcClient,
    c2s: Channel,
    s2c: Channel,
}

pub fn run_chaos(config: ChaosConfig) -> Result<ChaosSummary, ChaosFailure> {
    let mut rng = SplitMix64::new(config.seed);
    let store = Rc::new(RefCell::new(HashMap::new()));
    let exec_log: ExecLog = Rc::new(RefCell::new(Vec::new()));

    let mut links: Vec<ClientLink> = Vec::new();
    let mut servers: Vec<RpcServer<'static>> = Vec::new();
    let mut pending_syn: Vec<Vec<u8>> = Vec::new();

    for i in 0..config.clients {
        let (client_conn, syn) = Connection::connect(transport_config(), Tick(0));
        let server_conn = Connection::listen(transport_config(), Tick(0));
        let log = exec_log.clone();
        let store_clone = store.clone();
        let server = make_server(server_conn, store_clone, move |op, ok, resp| {
            log.borrow_mut().push((op.clone(), ok, resp.to_vec()));
        });
        let (c2s, s2c) = make_link(
            config.seed.wrapping_add(i as u64 * 1000),
            config.initial_profile,
        );
        links.push(ClientLink {
            client: RpcClient::new(client_conn, 25, 60),
            c2s,
            s2c,
        });
        servers.push(server);
        pending_syn.push(syn);
    }
    for (i, syn) in pending_syn.into_iter().enumerate() {
        links[i].c2s.send(&syn);
    }

    // Schedule (fire_at_tick, client_idx, op) in increasing tick order.
    let mut schedule: Vec<(u64, usize, Op)> = Vec::with_capacity(config.calls);
    let mut t = 1u64;
    for _ in 0..config.calls {
        t += 1 + rng.below(4) as u64;
        let client_idx = rng.below(config.clients);
        let key = format!("k{}", rng.below(config.key_space)).into_bytes();
        let op = match rng.below(3) {
            0 => Op::Get { key },
            1 => {
                let mut value = vec![0u8; 1 + rng.below(8)];
                for b in &mut value {
                    *b = rng.next_u64() as u8;
                }
                Op::Put { key, value }
            }
            _ => Op::Delete { key },
        };
        schedule.push((t, client_idx, op));
    }
    let mut switches = config.profile_switches.clone();
    switches.sort_by_key(|(t, _)| *t);

    let mut fired = 0usize;
    let mut switched = 0usize;
    let mut expected_completions: Vec<u64> = Vec::new();
    let mut calls_failed = 0usize;
    let mut tick = 0u64;

    loop {
        tick += 1;
        let now = Tick(tick);

        while switched < switches.len() && switches[switched].0 <= tick {
            let (_, profile) = switches[switched];
            for (i, link) in links.iter_mut().enumerate() {
                let (c2s, s2c) = make_link(
                    config
                        .seed
                        .wrapping_add(i as u64 * 1000)
                        .wrapping_add(switched as u64 * 7919),
                    profile,
                );
                link.c2s = c2s;
                link.s2c = s2c;
            }
            switched += 1;
        }

        while fired < schedule.len() && schedule[fired].0 <= tick {
            let (_, client_idx, op) = &schedule[fired];
            let (id, out) = call(&mut links[*client_idx].client, now, op);
            for d in out {
                links[*client_idx].c2s.send(&d);
            }
            expected_completions.push(id);
            fired += 1;
        }

        for (link, server) in links.iter_mut().zip(servers.iter_mut()) {
            for d in link.c2s.advance(now) {
                for out in server.on_datagram(now, &d) {
                    link.s2c.send(&out);
                }
            }
            for d in link.s2c.advance(now) {
                for out in link.client.on_datagram(now, &d) {
                    link.c2s.send(&out);
                }
            }
            for out in link.client.on_tick(now) {
                link.c2s.send(&out);
            }
            for out in server.on_tick(now) {
                link.s2c.send(&out);
            }
        }

        for link in &mut links {
            link.client.poll_completed(); // drain; the differential check below is what matters
                                          // A call can legitimately fail outright under a harsh enough
                                          // profile (e.g. `near_partition`'s 60% loss) — bounded
                                          // retries giving up is correct behavior there, not a chaos
                                          // invariant violation (the same lesson ADR-003 already
                                          // learned about this transport). Counted, not treated as an
                                          // error; the differential check below is what actually
                                          // matters, and it still runs over whatever *did* execute.
            calls_failed += link.client.poll_failed().len();
        }

        let all_pending_drained =
            fired == schedule.len() && links.iter().all(|l| l.client.pending_count() == 0);
        if all_pending_drained {
            break;
        }
        if tick >= config.max_ticks {
            return Err(ChaosFailure {
                seed: config.seed,
                message: format!(
                    "did not drain within {} ticks ({} of {} calls issued)",
                    config.max_ticks, fired, config.calls
                ),
            });
        }
    }

    // Differential check: replay the real server's exact execution order
    // against an independently-written reference store.
    let mut reference = ReferenceStore::new();
    let log = exec_log.borrow();
    for (i, (op, ok, payload)) in log.iter().enumerate() {
        let (ref_ok, ref_payload) = reference.apply(op);
        if *ok != ref_ok || payload != &ref_payload {
            return Err(ChaosFailure {
                seed: config.seed,
                message: format!(
                    "execution {i} ({op:?}) produced (ok={ok}, payload={payload:?}), reference store says (ok={ref_ok}, payload={ref_payload:?})"
                ),
            });
        }
    }

    Ok(ChaosSummary {
        seed: config.seed,
        executions: log.len(),
        calls_completed: expected_completions.len() - calls_failed,
        calls_failed,
        ticks_used: tick,
    })
}
