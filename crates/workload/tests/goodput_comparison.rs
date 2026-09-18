//! Ticket 006's other measurement: is distrans's own transport (ADR-003:
//! selective-repeat + SACK; ADR-004: AIMD) actually better than the two
//! textbook schemes it's built to improve on, and by how much? Not a
//! correctness test — a report (`cargo test -p workload --release --test
//! goodput_comparison -- --nocapture`), whose numbers are pasted into
//! ADR-006 rather than asserted on, since "our scheme wins" is exactly
//! the kind of claim this project's culture insists on measuring instead
//! of assuming.

use channel::{Channel, FaultProfile, Tick as ChannelTick};
use transport::{Config, Connection, State, Tick as TransportTick};
use workload::baselines::{run_go_back_n, run_stop_and_wait, ArqConfig};

const SEGMENT_SIZE: usize = 64;
const PAYLOAD_LEN: usize = 20_000;

fn profile_at(loss: f64) -> FaultProfile {
    FaultProfile {
        loss,
        duplication: 0.0,
        reorder_max_delay: 1,
        corruption: 0.0,
        truncation: 0.0,
        base_delay: 1,
    }
}

/// Runs one client->server bulk transfer over `transport::Connection`
/// and returns (ticks used, bytes delivered, completed).
fn run_transport(loss: f64, seed: u64) -> (u64, usize, bool) {
    let config = Config {
        segment_size: SEGMENT_SIZE,
        recv_window_capacity: 16 * SEGMENT_SIZE as u32,
        min_rto: 2,
        max_rto: 400,
        max_retries: 30,
        fast_retransmit_dup_acks: 3,
    };
    let profile = profile_at(loss);
    let (mut client, syn) = Connection::connect(config, TransportTick(0));
    let mut server = Connection::listen(config, TransportTick(0));
    let mut c2s = Channel::new(seed, profile);
    let mut s2c = Channel::new(seed.wrapping_add(9999), profile);
    c2s.send(&syn);

    let data = vec![3u8; PAYLOAD_LEN];
    let mut sent = false;
    let mut closed_client = false;
    let mut delivered = Vec::new();
    let max_ticks = 300_000u64;

    for tick in 1..=max_ticks {
        let now = ChannelTick(tick);
        let tnow = TransportTick(tick);
        for d in c2s.advance(now) {
            for out in server.on_datagram(tnow, &d) {
                s2c.send(&out);
            }
        }
        for d in s2c.advance(now) {
            for out in client.on_datagram(tnow, &d) {
                c2s.send(&out);
            }
        }
        for out in client.on_tick(tnow) {
            c2s.send(&out);
        }
        for out in server.on_tick(tnow) {
            s2c.send(&out);
        }
        delivered.extend(server.recv());

        if client.state() == State::Established && !sent {
            for out in client.send(tnow, &data) {
                c2s.send(&out);
            }
            sent = true;
        }
        if sent && !closed_client && client.state() == State::Established {
            for out in client.close(tnow) {
                c2s.send(&out);
            }
            closed_client = true;
        }
        if server.state() == State::Established && server.peer_closed() {
            for out in server.close(tnow) {
                s2c.send(&out);
            }
        }

        let done = client.state() == State::Closed && server.state() == State::Closed;
        let failed = matches!(client.state(), State::Failed(_))
            || matches!(server.state(), State::Failed(_));
        if done {
            return (tick, delivered.len(), true);
        }
        if failed {
            return (tick, delivered.len(), false);
        }
    }
    (max_ticks, delivered.len(), false)
}

#[test]
fn report_goodput_vs_loss_across_schemes() {
    let cfg = ArqConfig {
        segment_size: SEGMENT_SIZE,
        rto: 30, // fixed RTO for the reference baselines, see baselines.rs
        max_retries: 60,
    };
    let data = vec![3u8; PAYLOAD_LEN];
    let goodput = |ticks: u64, bytes: usize| bytes as f64 / ticks.max(1) as f64;

    // A truly ideal channel (no loss, no reordering at all) first, to show
    // go-back-N's pipelining actually winning when nothing is out of
    // order — the point of comparison this test exists to make.
    let ideal = FaultProfile::CLEAN;
    let mut fwd = Channel::new(1, ideal);
    let mut back = Channel::new(2, ideal);
    let saw = run_stop_and_wait(&data, &mut fwd, &mut back, cfg, 500_000);
    let mut fwd = Channel::new(1, ideal);
    let mut back = Channel::new(2, ideal);
    let gbn = run_go_back_n(&data, &mut fwd, &mut back, cfg, 8, 500_000);
    println!(
        "ideal (no loss, no reorder)  stop-and-wait: {:>6} ticks {:>7.3} B/t   go-back-n(w=8): {:>6} ticks {:>7.3} B/t",
        saw.ticks_used, goodput(saw.ticks_used, saw.delivered.len()),
        gbn.ticks_used, goodput(gbn.ticks_used, gbn.delivered.len()),
    );

    // Then the loss sweep, at a small but nonzero amount of reordering
    // (1 tick of jitter) even at loss=0 — a realistic touch of network
    // noise, and (see ADR-006) the thing that turns out to matter far
    // more than loss for go-back-N specifically.
    for loss in [0.0, 0.02, 0.05, 0.1, 0.2] {
        let profile = profile_at(loss);

        let mut fwd = Channel::new(1, profile);
        let mut back = Channel::new(2, profile);
        let saw = run_stop_and_wait(&data, &mut fwd, &mut back, cfg, 500_000);

        let mut fwd = Channel::new(1, profile);
        let mut back = Channel::new(2, profile);
        let gbn = run_go_back_n(&data, &mut fwd, &mut back, cfg, 8, 500_000);

        let (t_ticks, t_bytes, t_ok) = run_transport(loss, 1);

        println!(
            "loss={loss:>4.2}  stop-and-wait: {:>7} ticks {:>7.3} B/t (ok={})   go-back-n(w=8): {:>7} ticks {:>7.3} B/t (ok={})   distrans: {:>7} ticks {:>7.3} B/t (ok={t_ok})",
            saw.ticks_used, goodput(saw.ticks_used, saw.delivered.len()), saw.completed,
            gbn.ticks_used, goodput(gbn.ticks_used, gbn.delivered.len()), gbn.completed,
            t_ticks, goodput(t_ticks, t_bytes),
        );
    }
}
