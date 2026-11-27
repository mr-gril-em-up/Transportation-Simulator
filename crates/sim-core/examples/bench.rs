//! Times the baseline and a line edit on a synthetic Travis-County-sized region.
//! `cargo run --release -p sim-core --example bench [road_spacing_km]`
use sim_core::synth::{generate, straight_line, SynthConfig};
use sim_core::{Mode, ModeSplit, Params, Sim};
use std::time::Instant;

fn main() {
    let spacing: f64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.5);
    let cfg = SynthConfig {
        road_spacing_km: spacing,
        ..Default::default()
    };
    let input = generate(&cfg);
    println!(
        "zones {}  road nodes {}  links {}  commute pairs {}  threads {}",
        input.zones.len(),
        input.road_nodes.len(),
        input.road_links.len(),
        input.commutes.len(),
        rayon::current_num_threads()
    );
    let t = Instant::now();
    let mut sim = Sim::new(input, Params::default()).unwrap();
    println!("baseline equilibrium: {:.2} s", t.elapsed().as_secs_f64());
    let r = sim.results();
    println!(
        "daily trips {:.0}  shares {:?}",
        r.daily_trips.total(),
        r.mode_share
    );
    println!("peak avg speed {:.1} km/h", r.peak_avg_speed_kph);

    // Roughly ACS Travis County commute shares, applied to all trips for the demo.
    let t = Instant::now();
    sim.calibrate_mode_shares(
        ModeSplit {
            drive: 0.74,
            carpool: 0.09,
            transit_walk: 0.0,
            transit_drive: 0.0,
            walk: 0.05,
            bike: 0.02,
        },
        8,
    );
    println!(
        "calibrate: {:.2} s  shares {:?}",
        t.elapsed().as_secs_f64(),
        sim.results().mode_share
    );

    // A north-south light rail line through downtown to the Domain.
    let line = straight_line(Mode::LightRail, (0.5, -12.0), (2.0, 16.0), 1.2, 8.0);
    let t = Instant::now();
    let pv = sim.preview_line(line.clone()).unwrap();
    println!(
        "preview_line: {:.3} s  capital ${:.0}M  federal grant ${:.0}M  ${:.0}k per daily boarding",
        t.elapsed().as_secs_f64(),
        pv.capital_cost / 1e6,
        pv.federal_grant / 1e6,
        pv.capital_per_daily_boarding / 1e3
    );
    let t = Instant::now();
    let id = sim.add_line(line).unwrap();
    let dt = t.elapsed().as_secs_f64();
    let lr = &sim.results().lines[0];
    println!(
        "add_line: {:.3} s  id {id}  daily boardings {:.0}  peak load {:.0}/{:.0}  farebox {:.0}%",
        dt,
        lr.daily_boardings,
        lr.peak_hour_load,
        lr.peak_hour_capacity,
        lr.farebox_recovery * 100.0
    );
    let bus = straight_line(Mode::Bus, (-15.0, 3.0), (15.0, 3.0), 0.6, 12.0);
    let t = Instant::now();
    sim.add_line(bus).unwrap();
    println!(
        "add second line: {:.3} s  transit share {:.2}%",
        t.elapsed().as_secs_f64(),
        sim.results().mode_share.transit() * 100.0
    );
    let t = Instant::now();
    sim.equilibrate_warm();
    println!(
        "daily re-run (equilibrate_warm): {:.2} s",
        t.elapsed().as_secs_f64()
    );
    let t = Instant::now();
    sim.equilibrate();
    println!(
        "full re-run from free flow (equilibrate): {:.2} s",
        t.elapsed().as_secs_f64()
    );
    assert!(dt < 2.0, "line edit must re-evaluate in under 2 s");
    sim.skip_to_next_month();
    let m = sim.ledger().history.last().unwrap();
    println!(
        "month settled: fares ${:.1}M  op cost ${:.1}M  assistance ${:.1}M  local tax ${:.1}M (served pop {:.0})  cash ${:.0}M",
        m.fare_revenue / 1e6,
        m.operating_cost / 1e6,
        m.operating_assistance / 1e6,
        m.local_tax / 1e6,
        sim.ledger().served_population,
        sim.ledger().cash / 1e6
    );
}
