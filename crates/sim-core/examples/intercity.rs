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
//!
//! Mode choice (SIM-25): fits `asc_air` so the same market flies the FEIS
//! 2017 air share (5.6%), then draws a Dallas-Houston high-speed line on the
//! Texas Central alignment (Dallas, Brazos Valley, Houston; 30 min peak,
//! 60 min off-peak) and reports its share of the market with `asc_rail =
//! asc_air`. The FEIS forecast to compare: 6.5M riders in 2029, 29% of the
//! market. That share is a check, not a fit target.
use sim_core::{IntercityParams, Mode, Params, ScenarioInput, Sim, Stop, TransitLine};

const TARGET_PER_DAY: f64 = 15.97e6 / 365.0;
const AIR_SHARE: f64 = 0.056;

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
        instant_build: true,
        ..Params::default()
    };
    let scale = params.intercity.scale;
    let ip = params.intercity.clone();
    let mut sim = Sim::new(input, params).unwrap();
    let pairs = sim.intercity_pairs();
    let north = |p: [f64; 2]| near(p, (-96.797, 32.777), 60.0) || near(p, (-97.331, 32.755), 45.0);
    let houston = |p: [f64; 2]| near(p, (-95.369, 29.760), 70.0);
    let is_dh = |q: &sim_core::IntercityPair| {
        (north(q.from) && houston(q.to)) || (houston(q.from) && north(q.to))
    };
    let dh: f64 = pairs.iter().filter(|q| is_dh(q)).map(|q| q.trips).sum();
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

    // Trip-weighted shares over the Dallas-Houston pairs at a given air constant.
    let dh_shares = |pairs: &[sim_core::IntercityPair], asc: f64| -> [f64; 3] {
        let p = IntercityParams {
            asc_air: asc,
            asc_rail: asc,
            ..ip.clone()
        };
        let mut sum = [0.0; 3];
        for q in pairs.iter().filter(|q| is_dh(q)) {
            let s = p.shares(
                q.car_gc,
                q.air_gc.unwrap_or(f64::INFINITY),
                q.rail_gc.unwrap_or(f64::INFINITY),
            );
            (0..3).for_each(|m| sum[m] += q.trips * s[m]);
        }
        let t: f64 = sum.iter().sum();
        sum.map(|x| x / t)
    };
    let (mut lo, mut hi) = (-1000.0, 1000.0);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if dh_shares(&pairs, mid)[1] > AIR_SHARE {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let asc = 0.5 * (lo + hi);
    let gc = |pairs: &[sim_core::IntercityPair],
              f: &dyn Fn(&sim_core::IntercityPair) -> Option<f64>| {
        let (mut w, mut s) = (0.0, 0.0);
        for q in pairs.iter().filter(|q| is_dh(q)) {
            if let Some(g) = f(q) {
                w += q.trips;
                s += q.trips * g;
            }
        }
        s / w
    };
    println!(
        "Dallas-Houston mean generalized min: car {:.0}, air {:.0} (no constant)",
        gc(&pairs, &|q| Some(q.car_gc)),
        gc(&pairs, &|q| q.air_gc)
    );
    println!(
        "fitted asc_air {asc:.1} min (air share {:.3} vs FEIS {AIR_SHARE})",
        dh_shares(&pairs, asc)[1]
    );

    let stop = |name: &str, lon, lat| Stop {
        lon,
        lat,
        name: name.into(),
        park_ride: true,
        park_ride_spaces: None,
    };
    let hsr = TransitLine {
        name: "Dallas-Houston HSR".into(),
        mode: Mode::HighSpeedRail,
        stops: vec![
            stop("Dallas", -96.7935, 32.7693),
            stop("Brazos Valley", -96.000, 30.550),
            stop("Houston", -95.4578, 29.8091),
        ],
        headway_min: 30.0,
        offpeak_headway_min: Some(60.0),
        fare: None,
        fare_per_km: None,
        speed_kph: None,
        span_hours: None,
    };
    sim.add_line(hsr).unwrap();
    let pairs = sim.intercity_pairs();
    let s = dh_shares(&pairs, asc);
    println!(
        "with Dallas-Houston HSR: rail {:.3}, air {:.3}, car {:.3} of the market (FEIS 2029: rail 0.29); rail generalized min {:.0}",
        s[2],
        s[1],
        s[0],
        gc(&pairs, &|q| q.rail_gc)
    );
    let line = sim.results().lines.last().unwrap();
    println!(
        "HSR line at the default constants: {:.0} boardings/day ({:.1}M/yr), {:.0} intercity rail trips/day statewide",
        line.daily_boardings,
        line.daily_boardings * 365.0 / 1e6,
        sim.results().intercity_rail_trips
    );
}
