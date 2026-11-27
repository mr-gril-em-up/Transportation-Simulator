//! Defaults that come from sourced figures must match
//! `crates/data-pipeline/reference/figures.csv` (decision D25: sourced numbers
//! live in one file; sim-core copies them as constants and this test stops drift).

use sim_core::params::mode_defaults;
use sim_core::{Mode, Params};
use std::collections::HashMap;

fn figures() -> HashMap<String, f64> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../data-pipeline/reference/figures.csv"
    );
    let text = std::fs::read_to_string(path).expect("figures.csv is in the repo");
    text.lines()
        .skip(1)
        .filter_map(|l| {
            // Only the first two columns are read, so commas in notes do not matter.
            let mut cols = l.splitn(3, ',');
            let key = cols.next()?.to_string();
            let value = cols.next()?.parse().ok()?;
            Some((key, value))
        })
        .collect()
}

fn check(f: &HashMap<String, f64>, key: &str, ours: f64) {
    let theirs = *f
        .get(key)
        .unwrap_or_else(|| panic!("{key} missing from figures.csv"));
    assert!(
        (theirs - ours).abs() < 1e-9,
        "{key}: figures.csv says {theirs}, sim-core default is {ours}"
    );
}

#[test]
fn sourced_defaults_match_figures_csv() {
    let f = figures();
    let p = Params::default();
    check(&f, "trips_per_person_day_2022", p.trips_per_person);
    check(&f, "walk_wait_time_multiplier", p.wait_weight);
    check(&f, "walk_wait_time_multiplier", p.walk_weight);

    let bus = mode_defaults(Mode::Bus);
    let lrt = mode_defaults(Mode::LightRail);
    let crt = mode_defaults(Mode::CommuterRail);
    check(&f, "opex_bus_per_rev_hour", bus.cost_per_car_hour);
    check(&f, "opex_light_rail_per_rev_hour", lrt.cost_per_car_hour);
    check(&f, "opex_commuter_rail_per_rev_hour", crt.cost_per_car_hour);
    check(&f, "fare_capmetro_local", bus.fare);
    check(&f, "fare_capmetro_local", lrt.fare);
    check(&f, "fare_capmetro_commuter", crt.fare);
}
