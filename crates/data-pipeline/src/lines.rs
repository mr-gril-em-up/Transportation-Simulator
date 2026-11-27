//! Today's transit network as sim-core lines (DATA-7, DATA-9).
//!
//! sim-core treats `ScenarioInput.lines` as existing service: no capital
//! charge, open from day 0. The pipeline builds one line per GTFS route of
//! every agency feed in `reference/gtfs_feeds.csv` (`gtfs.rs`), writes them
//! to `existing_lines.json`, and puts them in `scenario.json` only with
//! `--existing-lines`, so calibration and the app's "start with today's
//! network" setting share one network.
//!
//! - Stops: the route's longest trip on the feed's busiest Wednesday. GTFS
//!   rather than OSM, because OSM maps only a handful of Houston METRO and
//!   VIA routes, while every agency publishes its full timetable.
//! - Mode from GTFS `route_type` (0 light rail, or tram when the name says
//!   streetcar or trolley; 1 metro; 2 commuter rail; 3 bus; 5 tram;
//!   11 trolleybus; 12 monorail; and the extended 100-999 ranges). Ferries,
//!   cable cars and funiculars are left out.
//!   `MODE` fixes feeds that code a route wrongly (CapMetro 550 and TEXRail
//!   are type 0).
//! - Name: agency, short name and long name, the short name left out when
//!   the long name already has it (`CapMetro 7 Duval/Dove Springs`,
//!   `DART Red Line`).
//! - Peak and midday headway and span from GTFS. Where `figures.csv` has
//!   timetable figures for a route (CapMetro's Red Line, 550), those win,
//!   because a count-based GTFS mean is coarse for a line with three peak
//!   trains.
//! - CapMetro rail stops named like a park-and-ride lot in `figures.csv`
//!   (`pnr_spaces_capmetro_<station>`) get `park_ride` and
//!   `park_ride_spaces`.
//! - A line is kept when any of its stops is inside the build's bounding box,
//!   so a Travis build gets CapMetro and a statewide build gets every agency.
//!
//! Lines are built through JSON so the pipeline does not depend on optional
//! sim-core fields existing in a given version.

use crate::gtfs::{Feed, RouteHeadway};
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

/// Timetable overrides by (feed id, route key): (peak, midday, span) figure keys.
const TIMETABLE: &[(&str, &str, [&str; 3])] = &[(
    "capmetro",
    "550",
    [
        "headway_capmetro_rail_peak_min",
        "headway_capmetro_rail_midday_min",
        "span_capmetro_rail_weekday_hours",
    ],
)];

/// Mode overrides by (feed id, route key) where a feed's `route_type` is
/// wrong: CapMetro codes the Red Line (550) and Trinity Metro codes TEXRail
/// as type 0 (light rail); both are diesel commuter rail on freight-shared
/// track.
const MODE: &[(&str, &str, &str)] = &[
    ("capmetro", "550", "commuter_rail"),
    ("trinity", "TEXRail", "commuter_rail"),
];

/// sim-core mode for a GTFS `route_type`; `name` tells a streetcar from
/// light rail, which share type 0.
fn mode(route_type: u16, name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    let street = n.contains("streetcar") || n.contains("trolley");
    Some(match route_type {
        0 | 900..=999 if street => "tram",
        0 => "light_rail",
        900..=999 => "tram",
        12 | 405 => "monorail",
        1 | 400..=499 => "metro",
        2 | 100..=199 => "commuter_rail",
        3 | 200..=299 | 700..=799 => "bus",
        5 => "tram",
        11 | 800 => "trolleybus",
        _ => return None,
    })
}

/// Park-and-ride spaces for a CapMetro rail stop name, from `pnr_spaces_capmetro_*`.
fn pnr_spaces(stop: &str) -> Option<f64> {
    let s = stop.to_lowercase();
    ["leander", "lakeline", "howard"]
        .iter()
        .find(|k| s.contains(*k))
        .and_then(|k| figure(&format!("pnr_spaces_capmetro_{k}")))
}

/// Existing lines for `scenario.json`: every feed's routes with headways and
/// at least two stops, any stop inside `bbox` (min lon, min lat, max lon,
/// max lat). Sorted by feed order, then route key.
pub fn existing_lines(
    feeds: &[(Feed, HashMap<String, RouteHeadway>)],
    bbox: &[f64; 4],
) -> Vec<TransitLine> {
    let inside =
        |lon: f64, lat: f64| lon >= bbox[0] && lat >= bbox[1] && lon <= bbox[2] && lat <= bbox[3];
    let mut out = Vec::new();
    for (feed, hw) in feeds {
        let mut keys: Vec<&String> = hw.keys().collect();
        keys.sort();
        for k in keys {
            let r = &hw[k];
            let label = if r.long_name.is_empty() {
                k.clone()
            } else if r.long_name.to_lowercase().contains(&k.to_lowercase()) {
                r.long_name.clone()
            } else {
                format!("{k} {}", r.long_name)
            };
            let fixed = MODE
                .iter()
                .find(|(f, id, _)| *f == feed.id && id == k)
                .map(|x| x.2);
            let Some(m) = fixed.or_else(|| r.route_type.and_then(|t| mode(t, &label))) else {
                continue;
            };
            if r.stops.len() < 2 || !r.stops.iter().any(|s| inside(s.lon, s.lat)) {
                continue;
            }
            let tt = TIMETABLE
                .iter()
                .find(|(f, id, _)| *f == feed.id && id == k)
                .map(|(_, _, keys)| keys.map(figure));
            let (peak, mid, span) = match tt {
                Some([Some(p), Some(o), s]) => (p, Some(o), s.or(Some(r.span_hours))),
                _ => {
                    let Some(p) = r.peak_headway_min.or(r.midday_headway_min) else {
                        continue;
                    };
                    (p, r.midday_headway_min, Some(r.span_hours))
                }
            };
            let pnr = feed.id == "capmetro" && m == "commuter_rail";
            let stops: Vec<_> = r
                .stops
                .iter()
                .map(|s| {
                    let spaces = if pnr { pnr_spaces(&s.name) } else { None };
                    serde_json::json!({
                        "lon": s.lon, "lat": s.lat, "name": s.name,
                        "park_ride": spaces.is_some(),
                        "park_ride_spaces": spaces,
                    })
                })
                .collect();
            let line = serde_json::from_value(serde_json::json!({
                "name": format!("{} {label}", feed.agency), "mode": m, "stops": stops,
                "headway_min": peak, "offpeak_headway_min": mid, "span_hours": span,
            }));
            if let Ok(l) = line {
                out.push(l);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gtfs::GtfsStop;

    fn route(ty: u16, long: &str, names: &[&str], peak: Option<f64>) -> RouteHeadway {
        RouteHeadway {
            peak_headway_min: peak,
            midday_headway_min: peak.map(|p| p * 2.0),
            span_hours: 18.0,
            trips: 10,
            long_name: long.into(),
            route_type: Some(ty),
            stops: names
                .iter()
                .enumerate()
                .map(|(i, n)| GtfsStop {
                    name: n.to_string(),
                    lon: -97.7 + i as f64 * 0.01,
                    lat: 30.3,
                })
                .collect(),
        }
    }

    fn feed(id: &str, agency: &str) -> Feed {
        Feed {
            id: id.into(),
            agency: agency.into(),
        }
    }

    #[test]
    fn red_line_uses_timetable_and_park_and_ride() {
        let cap: HashMap<String, RouteHeadway> = [
            (
                "7",
                route(3, "Duval/Dove Springs", &["A", "B", "C"], Some(15.0)),
            ),
            (
                "550",
                route(
                    0,
                    "MetroRail Red Line",
                    &["Leander Station", "Lakeline Station", "Downtown"],
                    Some(40.0),
                ),
            ),
            ("999", route(3, "", &["X", "Y"], None)),
            ("1", route(3, "", &["X"], Some(10.0))),
            ("F", route(4, "Ferry", &["X", "Y"], Some(10.0))),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let dart: HashMap<String, RouteHeadway> = [
            ("RED", route(0, "Red Line", &["P", "Q"], Some(15.0))),
            ("620", route(0, "Dallas Streetcar", &["S", "T"], Some(20.0))),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let tm: HashMap<String, RouteHeadway> = [(
            "TEXRail".to_string(),
            route(0, "TEXRail", &["U", "V"], Some(30.0)),
        )]
        .into();
        let feeds = vec![
            (feed("capmetro", "CapMetro"), cap),
            (feed("dart", "DART"), dart),
            (feed("trinity", "Trinity Metro"), tm),
        ];
        let bbox = [-98.0, 30.0, -97.0, 31.0];
        let lines = existing_lines(&feeds, &bbox);
        let v = serde_json::to_value(&lines).unwrap();
        let names: Vec<&str> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "CapMetro 550 MetroRail Red Line",
                "CapMetro 7 Duval/Dove Springs",
                "DART 620 Dallas Streetcar",
                "DART Red Line",
                "Trinity Metro TEXRail",
            ],
            "no-headway, one-stop and ferry routes dropped"
        );
        assert_eq!(v[0]["mode"], "commuter_rail");
        assert_eq!(v[0]["headway_min"], 32.0);
        assert_eq!(v[0]["offpeak_headway_min"], 51.0);
        assert_eq!(v[0]["stops"][0]["park_ride"], true);
        assert_eq!(v[0]["stops"][2]["park_ride"], false);
        assert_eq!(v[1]["stops"].as_array().unwrap().len(), 3);
        assert_eq!(v[1]["offpeak_headway_min"], 30.0);
        assert_eq!(v[2]["mode"], "tram");
        assert_eq!(v[3]["mode"], "light_rail");
        assert_eq!(v[4]["mode"], "commuter_rail", "TEXRail override");
        assert_eq!(figure("pnr_spaces_capmetro_lakeline"), Some(935.0));
        // Outside the box: nothing.
        assert!(existing_lines(&feeds, &[0.0, 0.0, 1.0, 1.0]).is_empty());
    }
}
