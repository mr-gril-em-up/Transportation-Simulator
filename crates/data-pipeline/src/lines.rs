//! Today's transit network as sim-core lines (DATA-7).
//!
//! sim-core treats `ScenarioInput.lines` as existing service: no capital
//! charge, open from day 0. The pipeline writes every OSM transit route that
//! got GTFS headways (`gtfs.rs`), one line per route `ref`, so the game can
//! start with today's CapMetro network and calibration fits the same network.
//! The app drops `lines` when the "start with today's network" setting is off.
//!
//! - One line per `ref`: the OSM direction relation with the most stops.
//! - Mode from the OSM `route` value; `train` is commuter rail.
//! - Peak and midday headway and span from GTFS. Where `figures.csv` has
//!   timetable figures for a route (the Red Line, 550), those win, because a
//!   count-based GTFS mean is coarse for a line with three peak trains.
//! - Stops named like a park-and-ride lot in `figures.csv`
//!   (`pnr_spaces_capmetro_<station>`) on a rail line get `park_ride` and
//!   `park_ride_spaces`.
//!
//! Lines are built through JSON so the pipeline does not depend on optional
//! sim-core fields existing in a given version.

use crate::model::{Transit, TransitRoute};
use sim_core::TransitLine;
use std::collections::HashMap;

const FIGURES: &str = include_str!("../reference/figures.csv");

/// A numeric value from `reference/figures.csv` by key.
pub fn figure(key: &str) -> Option<f64> {
    let mut r = csv::Reader::from_reader(FIGURES.as_bytes());
    r.records()
        .filter_map(|x| x.ok())
        .find(|x| x.get(0) == Some(key))
        .and_then(|x| x.get(1)?.parse().ok())
}

/// Timetable overrides by route ref: (peak, midday, span) figure keys.
const TIMETABLE: &[(&str, [&str; 3])] = &[(
    "550",
    [
        "headway_capmetro_rail_peak_min",
        "headway_capmetro_rail_midday_min",
        "span_capmetro_rail_weekday_hours",
    ],
)];

fn mode(osm: &str) -> Option<&'static str> {
    Some(match osm {
        "bus" => "bus",
        "trolleybus" => "trolleybus",
        "tram" => "tram",
        "light_rail" => "light_rail",
        "train" => "commuter_rail",
        "subway" => "metro",
        "monorail" => "monorail",
        _ => return None,
    })
}

/// Park-and-ride spaces for a rail stop name, from `pnr_spaces_capmetro_*`.
fn pnr_spaces(stop: &str) -> Option<f64> {
    let s = stop.to_lowercase();
    ["leander", "lakeline", "howard"]
        .iter()
        .find(|k| s.contains(*k))
        .and_then(|k| figure(&format!("pnr_spaces_capmetro_{k}")))
}

/// Existing lines for `scenario.json`, sorted by ref.
pub fn existing_lines(transit: &Transit) -> Vec<TransitLine> {
    let mut best: HashMap<&str, &TransitRoute> = HashMap::new();
    for r in &transit.routes {
        let (Some(k), true) = (
            r.r#ref.as_deref(),
            r.peak_headway_min.or(r.midday_headway_min).is_some(),
        ) else {
            continue;
        };
        if r.stops.len() < 2 || mode(&r.mode).is_none() {
            continue;
        }
        let e = best.entry(k).or_insert(r);
        if r.stops.len() > e.stops.len() {
            *e = r;
        }
    }
    let mut refs: Vec<&str> = best.keys().copied().collect();
    refs.sort();
    refs.into_iter()
        .filter_map(|k| {
            let r = best[k];
            let m = mode(&r.mode)?;
            let tt = TIMETABLE
                .iter()
                .find(|(id, _)| *id == k)
                .map(|(_, keys)| keys.map(figure));
            let (peak, mid, span) = match tt {
                Some([Some(p), Some(o), s]) => (p, Some(o), s.or(r.span_hours)),
                _ => {
                    let p = r.peak_headway_min.or(r.midday_headway_min)?;
                    (p, r.midday_headway_min, r.span_hours)
                }
            };
            let rail = m != "bus" && m != "trolleybus";
            let stops: Vec<_> = r
                .stops
                .iter()
                .map(|s| {
                    let name = s.name.clone().unwrap_or_default();
                    let spaces = if rail { pnr_spaces(&name) } else { None };
                    serde_json::json!({
                        "lon": s.lon, "lat": s.lat, "name": name,
                        "park_ride": spaces.is_some(),
                        "park_ride_spaces": spaces,
                    })
                })
                .collect();
            let name = match &r.name {
                Some(n) if !n.contains(k) => format!("{k} {n}"),
                Some(n) => n.clone(),
                None => k.to_string(),
            };
            serde_json::from_value(serde_json::json!({
                "name": name, "mode": m, "stops": stops,
                "headway_min": peak, "offpeak_headway_min": mid, "span_hours": span,
            }))
            .ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Stop;

    fn route(r: &str, mode: &str, names: &[&str], peak: Option<f64>) -> TransitRoute {
        TransitRoute {
            osm_id: 1,
            mode: mode.into(),
            r#ref: Some(r.into()),
            name: None,
            operator: None,
            stops: names
                .iter()
                .enumerate()
                .map(|(i, n)| Stop {
                    osm_id: i as i64,
                    name: Some(n.to_string()),
                    lon: -97.7 + i as f64 * 0.01,
                    lat: 30.3,
                })
                .collect(),
            peak_headway_min: peak,
            midday_headway_min: peak.map(|p| p * 2.0),
            span_hours: Some(18.0),
        }
    }

    #[test]
    fn red_line_uses_timetable_and_park_and_ride() {
        let t = Transit {
            routes: vec![
                route("7", "bus", &["A", "B"], Some(15.0)),
                route("7", "bus", &["A", "B", "C"], Some(15.0)),
                route(
                    "550",
                    "train",
                    &["Leander Station", "Lakeline Station", "Downtown"],
                    Some(40.0),
                ),
                route("999", "bus", &["X", "Y"], None),
            ],
        };
        let lines = existing_lines(&t);
        assert_eq!(lines.len(), 2, "no-headway route dropped, one line per ref");
        let v = serde_json::to_value(&lines).unwrap();
        assert_eq!(v[0]["name"], "550");
        assert_eq!(v[0]["mode"], "commuter_rail");
        assert_eq!(v[0]["headway_min"], 32.0);
        assert_eq!(v[0]["offpeak_headway_min"], 51.0);
        assert_eq!(v[0]["stops"][0]["park_ride"], true);
        assert_eq!(v[0]["stops"][2]["park_ride"], false);
        assert_eq!(v[1]["stops"].as_array().unwrap().len(), 3);
        assert_eq!(v[1]["offpeak_headway_min"], 30.0);
        assert_eq!(figure("pnr_spaces_capmetro_lakeline"), Some(935.0));
    }
}
