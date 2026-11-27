//! F37: calibrate the intercity gravity scale to the Dallas-Houston market.
//!
//! `cargo run --release -p sim-core --example intercity -- <statewide scenario.json> [cutoff_min]`
//!
//! Target: FRA Dallas-Houston HSR FEIS (Appendix J, Table 8, 2017 base year):
//! 15.97M person trips a year between the North Texas and Houston
//! catchments, i.e. 43,753 a day both directions. The metros here are disks
//! around Dallas, Fort Worth and Houston (an approximation of the FEIS
//! catchments). Prints the scale that hits the target; paste it into
//! `IntercityParams::default`. Intercity demand is built on free-flow times, so
//! one assignment round is enough here (its road speeds mean nothing).
use sim_core::{Params, ScenarioInput, Sim};

const TARGET_PER_DAY: f64 = 15.97e6 / 365.0;

fn near(p: [f64; 2], c: (f64, f64), r_km: f64) -> bool {
    let k = 111.32;
    let dx = (p[0] - c.0) * k * (0.5 * (p[1] + c.1)).to_radians().cos();
    let dy = (p[1] - c.1) * k;
    (dx * dx + dy * dy).sqrt() <= r_km
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let input =
        ScenarioInput::from_json(&std::fs::read_to_string(&a[1]).expect("scenario.json")).unwrap();
    let cut: f64 = a.get(2).and_then(|c| c.parse().ok()).unwrap_or(60.0);
    let pop: f64 = input.zones.iter().map(|z| z.population).sum();
    let params = Params {
        strategic_max_min: Some(cut),
        feedback_iterations: 1,
        ..Params::default()
    };
    let scale = params.intercity.scale;
    let sim = Sim::new(input, params).unwrap();
    let pairs = sim.intercity_pairs();
    let north = |p: [f64; 2]| near(p, (-96.797, 32.777), 60.0) || near(p, (-97.331, 32.755), 45.0);
    let houston = |p: [f64; 2]| near(p, (-95.369, 29.760), 70.0);
    let dh: f64 = pairs
        .iter()
        .filter(|q| (north(q.from) && houston(q.to)) || (houston(q.from) && north(q.to)))
        .map(|q| q.trips)
        .sum();
    let total: f64 = pairs.iter().map(|q| q.trips).sum();
    let pkm: f64 = pairs.iter().map(|q| q.trips * q.car_km).sum();
    println!(
        "{} directed cell pairs past {cut} min; scale {scale:e}",
        pairs.len()
    );
    println!("Dallas-Houston {dh:.0}/day vs FEIS {TARGET_PER_DAY:.0}/day");
    let fit = scale * TARGET_PER_DAY / dh;
    println!("fitted scale {fit:.4e}");
    let k = fit / scale;
    println!(
        "at the fitted scale: {:.0} intercity trips/day statewide, {:.2} a resident a year, mean {:.0} km",
        total * k,
        total * k * 365.0 / pop,
        pkm / total
    );
}
