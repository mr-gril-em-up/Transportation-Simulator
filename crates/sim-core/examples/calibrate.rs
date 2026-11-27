//! F14 calibration: fit the mode-choice constants to Travis ACS commute
//! shares with today's CapMetro network loaded, then report the fit, the
//! fitted constants and Red Line ridership (F19).
//!
//! `cargo run --release -p sim-core --example calibrate -- <scenario.json> <transit.json> [bus_peak_headway] [bus_offpeak_headway]`
//!
//! The network comes from the build's `transit.json` (OSM route relations):
//! one line per route ref (OSM keeps each direction as its own relation),
//! the Red Line (ref 550) as commuter rail at 32/51 min, buses at the given
//! headways (default 20/30 min; OSM has no frequencies, GTFS is a follow-up).
//! Red Line park-and-ride runs only at the three stations with lots, capped
//! at their spaces (SIM-11).
//! Targets are ACS journey-to-work shares excluding work from home (`targets`).
//! Paste the printed `asc` into `Params::default` to adopt a new fit.
use sim_core::{Mode, ModeSplit, Params, ScenarioInput, Sim, Stop, TransitLine};
use std::collections::HashSet;
use std::time::Instant;

/// Travis County transit share of commuters (figures.csv
/// `acs_travis_commute_transit`, ACS 2019-2023 B08301, share of all workers).
const ACS_TRANSIT: f64 = 0.0167;

/// Red Line park-and-ride lots and spaces (figures.csv
/// `pnr_spaces_capmetro_*`, CapMetro; rows pending from the data stream).
/// The other stations have no lot.
const RED_LINE_LOTS: [(&str, f64); 3] =
    [("Leander", 619.0), ("Lakeline", 935.0), ("Howard", 301.0)];

/// ACS 2019-2023 B08301 Travis commute shares of all workers (figures.csv
/// `acs_travis_commute_*`). Work from home (27.2%) and taxi/motorcycle/other
/// (1.3%) are not modeled trips; the fit normalizes the rest.
fn targets() -> ModeSplit {
    ModeSplit {
        drive: 0.5973,
        carpool: 0.0746,
        // Transit fitted as one share: the lots' spaces, not a tuned
        // access split, set how many drive to the Red Line (SIM-11).
        transit_walk: ACS_TRANSIT,
        transit_drive: 0.0,
        walk: 0.0207,
        bike: 0.0060,
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut input =
        ScenarioInput::from_json(&std::fs::read_to_string(&a[1]).expect("scenario.json")).unwrap();
    let transit: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&a[2]).expect("transit.json")).unwrap();
    let bus_peak: f64 = a.get(3).and_then(|h| h.parse().ok()).unwrap_or(20.0);
    let bus_off: f64 = a.get(4).and_then(|h| h.parse().ok()).unwrap_or(30.0);

    let mut seen = HashSet::new();
    for r in transit["routes"].as_array().unwrap() {
        let reference = r["ref"].as_str().unwrap_or("").to_string();
        let stops: Vec<Stop> = r["stops"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| Stop {
                lon: s["lon"].as_f64().unwrap(),
                lat: s["lat"].as_f64().unwrap(),
                name: s["name"].as_str().unwrap_or("").into(),
                park_ride: false,
                park_ride_spaces: None,
            })
            .collect();
        if stops.len() < 2 || !seen.insert(reference.clone()) {
            continue;
        }
        let rail = reference == "550";
        input.lines.push(TransitLine {
            name: format!("CapMetro {reference}"),
            mode: if rail { Mode::CommuterRail } else { Mode::Bus },
            stops: if rail {
                stops
                    .into_iter()
                    .map(|s| {
                        let lot = RED_LINE_LOTS.iter().find(|(n, _)| *n == s.name);
                        Stop {
                            park_ride: lot.is_some(),
                            park_ride_spaces: lot.map(|l| l.1),
                            ..s
                        }
                    })
                    .collect()
            } else {
                stops
            },
            headway_min: if rail { 32.0 } else { bus_peak },
            offpeak_headway_min: Some(if rail { 51.0 } else { bus_off }),
            fare: None,
            fare_per_km: None,
            speed_kph: None,
            span_hours: None,
        });
    }
    let n_lines = input.lines.len();
    let t = Instant::now();
    let params = Params {
        instant_build: true,
        ..Params::default()
    };
    let mut sim = Sim::new(input, params).unwrap();
    println!(
        "{n_lines} CapMetro lines loaded (bus {bus_peak}/{bus_off} min): {:.1} s",
        t.elapsed().as_secs_f64()
    );
    report(&sim, "before");

    let t = Instant::now();
    for _ in 0..3 {
        sim.calibrate_mode_shares(targets(), 10);
        sim.equilibrate_warm();
    }
    sim.calibrate_mode_shares(targets(), 10);
    println!("calibrated in {:.1} s", t.elapsed().as_secs_f64());
    report(&sim, "after");
    println!(
        "asc {:?}",
        sim.params.asc.map(|x| (x * 1000.0).round() / 1000.0)
    );
}

fn report(sim: &Sim, label: &str) {
    let c = sim.commute_mode_shares();
    let pct = |x: f64| (x * 1000.0).round() / 10.0;
    let tg = targets().shares();
    println!(
        "{label}: commute drive {} carpool {} transit {} (walk access {}, drive access {}) walk {} bike {} (%, ACS target {} {} {} {} {})",
        pct(c.drive),
        pct(c.carpool),
        pct(c.transit()),
        pct(c.transit_walk),
        pct(c.transit_drive),
        pct(c.walk),
        pct(c.bike),
        pct(tg.drive),
        pct(tg.carpool),
        pct(tg.transit()),
        pct(tg.walk),
        pct(tg.bike)
    );
    let r = sim.results();
    let red: f64 = r
        .lines
        .iter()
        .filter(|l| l.mode == Mode::CommuterRail)
        .map(|l| l.daily_boardings)
        .sum();
    let bus: f64 = r
        .lines
        .iter()
        .filter(|l| l.mode == Mode::Bus)
        .map(|l| l.daily_boardings)
        .sum();
    println!(
        "  all trips transit {}%  Red Line {red:.0}/day  buses {bus:.0}/day",
        pct(r.mode_share.transit())
    );
    for l in r.lines.iter().filter(|l| l.mode == Mode::CommuterRail) {
        let cars: Vec<String> = l
            .park_ride_cars_by_stop
            .iter()
            .filter(|&&c| c > 0.0)
            .map(|c| format!("{c:.0}"))
            .collect();
        println!("  {} park-and-ride cars {}", l.name, cars.join(" / "));
    }
}
