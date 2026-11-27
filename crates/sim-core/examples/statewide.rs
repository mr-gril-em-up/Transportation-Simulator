//! F34: one game day for all of Texas on the strategic tier.
//! `cargo run --release -p sim-core --example statewide -- <scenario.json> [cutoff_min] [cold_rounds] [days]`
//! (the `data-48-2023-tract-highway` release; cutoff default 90 minutes,
//! `cold_rounds` sets `feedback_iterations`, default 4; `days` warm days, default 1).
//! Prints load time, the cold equilibrium (`Sim::new`), one warm game day
//! (`equilibrate_warm`, what `tick` runs daily, the 20 s target) and memory.
//! With `LINES=<existing_lines.json>` it starts from those lines and times a
//! line edit (`preview_line` and `add_line` of a copy of the first line; the
//! F18 target is 2 s).
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
    let mut input = input;
    let lines: Vec<sim_core::TransitLine> = std::env::var("LINES")
        .ok()
        .map(|f| serde_json::from_str(&std::fs::read_to_string(f).expect("LINES")).unwrap())
        .unwrap_or_default();
    input.lines = lines.clone();
    // ROADS=<file>: start warm from saved road volumes if the file exists,
    // and save them there at the end (SIM-19).
    let roads_file = std::env::var("ROADS").ok();
    let saved: Option<sim_core::RoadState> = roads_file
        .as_ref()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|j| serde_json::from_str(&j).ok());
    let mut sim = match &saved {
        Some(r) => Sim::new_warm(input, params, r).unwrap(),
        None => Sim::new(input, params).unwrap(),
    };
    if saved.is_some() {
        println!("warm start from saved roads: {}", sim.started_warm());
    }
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
            "day {d}: {warm:.1} s, peak speed {:.0} km/h, car-unreachable pairs {}, car pairs past the guard {}, unserved {:.0}",
            sim.results().peak_avg_speed_kph,
            sim.car_unreachable_pairs(),
            sim.car_past_guard_pairs(),
            sim.results().unserved_trips
        );
    }
    if let Some(l) = lines.first() {
        let mut l = l.clone();
        l.name = format!("{} copy", l.name);
        let t = Instant::now();
        let pv = sim.preview_line(l.clone()).unwrap();
        let preview = t.elapsed().as_secs_f64();
        let t = Instant::now();
        sim.add_line(l).unwrap();
        println!(
            "{} lines: preview {preview:.2} s ({:.0} boardings/day), add {:.2} s",
            lines.len(),
            pv.line.daily_boardings,
            t.elapsed().as_secs_f64()
        );
    }
    // One more day through `tick_budgeted` (SIM-18): the longest single call
    // is how long a line edit can wait behind the day.
    let t = Instant::now();
    sim.clock.set_timelapse(60.0);
    let budget = std::time::Duration::from_millis(100);
    let (mut calls, mut longest) = (0, 0.0f64);
    let mut step = |sim: &mut Sim, dt| {
        let c = Instant::now();
        sim.tick_budgeted(dt, budget);
        longest = longest.max(c.elapsed().as_secs_f64());
    };
    step(&mut sim, 24.0);
    calls += 1;
    while sim.day_progress().is_some() {
        step(&mut sim, 0.0);
        calls += 1;
    }
    println!(
        "budgeted day: {:.1} s in {calls} calls of 100 ms budget, longest call {longest:.2} s",
        t.elapsed().as_secs_f64()
    );
    if let Some(f) = &roads_file {
        std::fs::write(f, serde_json::to_string(&sim.road_state()).unwrap()).unwrap();
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
