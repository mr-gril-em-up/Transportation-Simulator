//! F34: one game day for all of Texas on the strategic tier.
//! `cargo run --release -p sim-core --example statewide -- <scenario.json> [cutoff_min] [cold_rounds] [days]`
//! (the `data-48-2023-tract-highway` release; cutoff default 90 minutes,
//! `cold_rounds` sets `feedback_iterations`, default 4; `days` warm days, default 1).
//! Prints load time, the cold equilibrium (`Sim::new`), one warm game day
//! (`equilibrate_warm`, what `tick` runs daily, the 20 s target) and memory.
use sim_core::{Params, ScenarioInput, Sim};
use std::time::Instant;

fn peak_gb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .map(|kb| kb / 1e6)
        .unwrap_or(0.0)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cut: f64 = a.get(2).and_then(|c| c.parse().ok()).unwrap_or(90.0);
    let t = Instant::now();
    let input =
        ScenarioInput::from_json(&std::fs::read_to_string(&a[1]).expect("scenario.json")).unwrap();
    println!(
        "loaded {} zones, {} road nodes, {} links, {} flows in {:.1} s",
        input.zones.len(),
        input.road_nodes.len(),
        input.road_links.len(),
        input.commutes.len(),
        t.elapsed().as_secs_f64()
    );
    let rounds: usize = a.get(3).and_then(|c| c.parse().ok()).unwrap_or(4);
    let days: usize = a.get(4).and_then(|c| c.parse().ok()).unwrap_or(1);
    let params = Params {
        strategic_max_min: Some(cut),
        feedback_iterations: rounds,
        ..Params::default()
    };
    let t = Instant::now();
    let mut sim = Sim::new(input, params).unwrap();
    let cold = t.elapsed().as_secs_f64();
    println!(
        "cold start ({rounds} rounds): {cold:.1} s, peak speed {:.0} km/h",
        sim.results().peak_avg_speed_kph
    );
    let mut warm = 0.0;
    for d in 1..=days {
        let t = Instant::now();
        sim.equilibrate_warm();
        warm = t.elapsed().as_secs_f64();
        println!(
            "day {d}: {warm:.1} s, peak speed {:.0} km/h, car-unreachable pairs {}",
            sim.results().peak_avg_speed_kph,
            sim.car_unreachable_pairs()
        );
    }
    let r = sim.results();
    let sh = r.mode_share;
    println!(
        "cutoff {cut} min: cold day {cold:.1} s, warm day {warm:.1} s, peak {:.2} GB",
        peak_gb()
    );
    println!(
        "trips {:.0}/day, beyond cutoff {:.0}, unserved {:.0}; drive {:.1}% carpool {:.1}% walk {:.1}% bike {:.1}%; peak speed {:.0} km/h",
        r.daily_trips.total(),
        r.beyond_cutoff_trips,
        r.unserved_trips,
        100.0 * sh.drive,
        100.0 * sh.carpool,
        100.0 * sh.walk,
        100.0 * sh.bike,
        r.peak_avg_speed_kph
    );
}
