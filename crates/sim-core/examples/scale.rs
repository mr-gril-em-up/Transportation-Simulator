//! F34 groundwork: how `Sim::new` (one full equilibrium day) scales with zone
//! count on a synthetic region. `cargo run --release -p sim-core --example scale -- 26 40 56`
//! Set `CUTOFF=<minutes>` to run with `Params::strategic_max_min`.
use sim_core::synth::{generate, SynthConfig};
use sim_core::{Params, Sim};
use std::time::Instant;

fn main() {
    for side in std::env::args()
        .skip(1)
        .map(|a| a.parse::<usize>().unwrap())
    {
        let cfg = SynthConfig {
            zones_per_side: side,
            extent_km: 2.0 * side as f64,
            road_spacing_km: 1.0,
            population: 50_000.0 * (side * side) as f64 / 26.0,
            jobs: 33_000.0 * (side * side) as f64 / 26.0,
            ..SynthConfig::default()
        };
        let input = generate(&cfg);
        let nodes = input.road_nodes.len();
        let t = Instant::now();
        let params = Params {
            strategic_max_min: std::env::var("CUTOFF").ok().and_then(|c| c.parse().ok()),
            ..Params::default()
        };
        let sim = Sim::new(input, params).unwrap();
        let secs = t.elapsed().as_secs_f64();
        let rss = std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("VmHWM"))
                    .map(|l| l.split_whitespace().nth(1).unwrap().parse::<f64>().unwrap() / 1e6)
            })
            .unwrap_or(0.0);
        println!(
            "zones {} nodes {nodes}: day {secs:.1} s, peak RSS {rss:.2} GB, trips {:.0}, beyond cutoff {:.0}",
            side * side,
            sim.results().daily_trips.total(),
            sim.results().beyond_cutoff_trips
        );
    }
}
