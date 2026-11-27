//! F19 calibration probe: the CapMetro Red Line (OSM route 550) on a real build.
//! `cargo run --release -p sim-core --example redline -- <scenario.json> <transit.json> [peak_headway] [offpeak_headway]`
//! Prints daily boardings per stop and the mode split, to compare with NTD
//! (`ridership_capmetro_rail_annual` in the pipeline's figures.csv).
use sim_core::{Mode, Params, Sim, Stop, TransitLine};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input = std::fs::read_to_string(&args[1]).expect("scenario.json");
    let transit: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&args[2]).expect("transit.json")).unwrap();
    let headway: f64 = args.get(3).and_then(|h| h.parse().ok()).unwrap_or(30.0);
    let route = transit["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["ref"] == "550")
        .expect("route 550 in transit.json");
    let stops: Vec<Stop> = route["stops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| Stop {
            lon: s["lon"].as_f64().unwrap(),
            lat: s["lat"].as_f64().unwrap(),
            name: s["name"].as_str().unwrap_or("").into(),
            park_ride: true,
        })
        .collect();
    let mut sim = Sim::from_json(&input, Params::default()).unwrap();
    let line = TransitLine {
        name: "Red Line".into(),
        mode: Mode::CommuterRail,
        stops: stops.clone(),
        headway_min: headway,
        offpeak_headway_min: args.get(4).and_then(|h| h.parse().ok()),
        fare: None,
        fare_per_km: None,
        speed_kph: None,
        span_hours: None,
    };
    sim.add_line(line).unwrap();
    let r = sim.results();
    let l = &r.lines[0];
    println!(
        "Red Line, {} stops, {:.1} km, {:.0} min, headway {headway} (off-peak {:?}): {:.0} boardings/day",
        stops.len(),
        l.length_km,
        l.run_min,
        args.get(4),
        l.daily_boardings
    );
    for (s, b) in stops.iter().zip(&l.boardings_by_stop) {
        println!("  {:<20} {:>7.0}", s.name, b);
    }
    println!("mode share {:?}", r.mode_share);
}
