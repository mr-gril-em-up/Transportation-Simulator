//! F38: compare modeled road volumes with TxDOT AADT counts.
//!
//! `cargo run --release -p sim-core --example counts -- <scenario.json> <counts.json> [cutoff_min] [rounds]`
//!
//! `counts.json` comes from the data pipeline (DATA, F38): TxDOT "AADT
//! Annuals" stations snapped to scenario links, each with `aadt` (two-way)
//! and the link's index in `road_links`, plus `link2` for the opposite
//! carriageway of a divided road; inactive stations are skipped. The model's volume at a station is the
//! peak-hour flow on every directed traversal of those links. The model has
//! peak-hour volumes and the counts are daily, so the report fits one K
//! (peak-hour share of daily traffic) over all stations and then shows how
//! each road class and volume band departs from it. Real K is ~0.08-0.10, so
//! a fitted K far below that means the model carries too little traffic.
//! `LINES=<existing_lines.json>` loads today's transit first, and
//! `ROADS=<file>` warm-starts from saved road volumes, as in `statewide.rs`.
use sim_core::{Params, ScenarioInput, Sim};
use std::collections::HashMap;

/// (AADT, model peak-hour two-way volume, class, road name).
type Row = (f64, f64, String, String);

#[derive(serde::Deserialize)]
struct Station {
    aadt: f64,
    /// Index into `road_links`.
    link: u32,
    /// Opposite carriageway of a divided road.
    link2: Option<u32>,
    #[serde(default)]
    on_road: String,
    #[serde(default = "yes")]
    active: bool,
}

fn yes() -> bool {
    true
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut input =
        ScenarioInput::from_json(&std::fs::read_to_string(&a[1]).expect("scenario.json")).unwrap();
    let text = std::fs::read_to_string(&a[2]).expect("counts.json");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let list = if v.is_array() {
        v
    } else {
        v["stations"].clone()
    };
    let stations: Vec<Station> = serde_json::from_value(list).expect("counts: station array");
    let cut: Option<f64> = a.get(3).and_then(|c| c.parse().ok());
    let rounds: usize = a.get(4).and_then(|c| c.parse().ok()).unwrap_or(4);
    if let Ok(f) = std::env::var("LINES") {
        input.lines = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
    }
    // Class per link, for the breakdown.
    let class: Vec<String> = input
        .road_links
        .iter()
        .map(|l| {
            l.class
                .clone()
                .unwrap_or_else(|| format!("{:.0} km/h", l.speed_kph))
        })
        .collect();
    let params = Params {
        strategic_max_min: cut,
        feedback_iterations: rounds,
        ..Params::default()
    };
    let saved: Option<sim_core::RoadState> = std::env::var("ROADS")
        .ok()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|j| serde_json::from_str(&j).ok());
    let sim = match &saved {
        Some(r) => Sim::new_warm(input, params, r).unwrap(),
        None => Sim::new(input, params).unwrap(),
    };
    // Both directions of each link, as AADT counts both.
    let mut vol: HashMap<u32, f64> = HashMap::new();
    for f in sim.link_flows() {
        *vol.entry(f.link).or_default() += f.volume_vph;
    }
    let mut rows: Vec<Row> = Vec::new();
    for s in &stations {
        if s.aadt <= 0.0 || !s.active {
            continue;
        }
        let m = vol.get(&s.link).copied().unwrap_or(0.0)
            + s.link2.map_or(0.0, |l| vol.get(&l).copied().unwrap_or(0.0));
        let c = class
            .get(s.link as usize)
            .cloned()
            .unwrap_or_else(|| "?".into());
        rows.push((s.aadt, m, c, s.on_road.clone()));
    }
    let (sum_c, sum_m): (f64, f64) = rows.iter().fold((0.0, 0.0), |a, r| (a.0 + r.0, a.1 + r.1));
    let k = sum_m / sum_c;
    println!(
        "{} stations ({} in file), fitted K (model peak hour / AADT) {k:.4}; real K is ~0.08-0.10",
        rows.len(),
        stations.len()
    );
    println!(
        "peak speed {:.1} km/h, {:.0} daily trips",
        sim.results().peak_avg_speed_kph,
        sim.results().daily_trips.total()
    );
    // Percent RMSE of model/K against AADT, and median ratio, per group.
    let report = |name: &str, sel: &dyn Fn(&Row) -> bool| {
        let g: Vec<_> = rows.iter().filter(|r| sel(r)).collect();
        if g.is_empty() {
            return;
        }
        let n = g.len() as f64;
        let mean_c = g.iter().map(|r| r.0).sum::<f64>() / n;
        let mse = g.iter().map(|r| (r.1 / k - r.0).powi(2)).sum::<f64>() / n;
        let mut ratio: Vec<f64> = g.iter().map(|r| r.1 / k / r.0).collect();
        ratio.sort_by(|a, b| a.total_cmp(b));
        let sum_ratio = g.iter().map(|r| r.1).sum::<f64>() / k / g.iter().map(|r| r.0).sum::<f64>();
        println!(
            "  {name:<22} n {:>6}  mean AADT {:>7.0}  model/count {:>5.2} (median {:>5.2})  %RMSE {:>4.0}",
            g.len(),
            mean_c,
            sum_ratio,
            ratio[ratio.len() / 2],
            100.0 * mse.sqrt() / mean_c
        );
    };
    println!("by AADT band (model scaled by the fitted K):");
    for (lo, hi) in [
        (0.0, 5e3),
        (5e3, 2e4),
        (2e4, 5e4),
        (5e4, 1e5),
        (1e5, f64::INFINITY),
    ] {
        report(&format!("{lo:.0}-{hi:.0}"), &|r| r.0 >= lo && r.0 < hi);
    }
    println!("by road class:");
    let mut classes: Vec<String> = rows.iter().map(|r| r.2.clone()).collect();
    classes.sort();
    classes.dedup();
    for c in classes {
        report(&c, &|r| r.2 == c);
    }
    let unloaded = rows.iter().filter(|r| r.1 == 0.0).count();
    println!("stations with zero model volume: {unloaded}");
    report("all", &|_| true);
}
