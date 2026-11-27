//! TxDOT traffic count stations snapped to scenario links (F38, DATA-10).
//!
//! Input: `raw/txdot_aadt.csv`, written by `scripts/txdot_aadt.py` from
//! TxDOT's "AADT Annuals" layer (one point per count station, latest annual
//! AADT, both directions combined). Output: `counts.json`, one entry per
//! station that snaps cleanly to a scenario road link, for calibrating car
//! assignment against counts.
//!
//! Snapping, per station:
//! - Candidates: `roads.json` edge segments (shape included) within
//!   [`MAX_M`] of the station that survived into `scenario.json`.
//! - The station's `ON_ROAD` must match the edge: route numbers compare as
//!   prefix and number (`IH0035` = `I 35`, `SH0071` = `TX 71`,
//!   `FM2222` = `RM 2222`, `SL0001` = `Loop 1`) against OSM `ref`; anything
//!   else compares as a street name against OSM `name`, with common suffixes
//!   (`BLVD`, `ST`, ...) normalized.
//! - The nearest matching edge gives `from`/`to`, the scenario link it
//!   belongs to. When that edge is one-way (a divided road), the nearest
//!   matching one-way edge heading the other way gives `from2`/`to2`, so the
//!   two-way AADT can be compared with the sum of both carriageways.
//! - Stations with no match within range are dropped.
//!
//! Station type comes from the TxDOT id suffix ([`Facility`]):
//! - `..NBSR`, `..SBSR`, `..EBSR`, `..WBSR`: a one-direction count on a
//!   frontage (service) road. It never snaps to freeway main lanes or ramps
//!   (`motorway`, `motorway_link`, `trunk_link`); it takes a matching
//!   non-freeway edge, or one named `... Frontage ...`/`... Service ...`,
//!   heading the suffix direction. No match: dropped.
//! - `..NBDR`, `..SBDR`, `..EBDR`, `..WBDR`: a one-direction count on a
//!   direct connector ramp. Only `*_link` edges heading that way qualify.
//! - Anything else: a main-lane count, snapped as above.
//!
//! Each count carries `external`: its TxDOT `CNTY_NM` maps (via
//! `reference/tx_counties.csv`) to a county outside `--area`. A county build
//! keeps roads a margin past the county line, so those stations snap but sit
//! where there are no zones.

use crate::census::Area;
use crate::model::Roads;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

/// Maximum station-to-road distance, meters.
pub const MAX_M: f64 = 100.0;

/// A TxDOT count station from `txdot_aadt.csv`.
#[derive(Debug, Clone, PartialEq)]
pub struct Station {
    pub id: String,
    pub year: u16,
    pub aadt: u32,
    pub on_road: String,
    pub lon: f64,
    pub lat: f64,
    pub active: bool,
    /// 5-digit county GEOID from `CNTY_NM`, if the name is known.
    pub county: Option<String>,
}

/// What a station counts, from its id suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Facility {
    /// Main lanes, both directions.
    Main,
    /// Frontage (service) road, one direction (`..NBSR`).
    Frontage,
    /// Direct connector ramp, one direction (`..NBDR`).
    Ramp,
}

/// Facility and compass direction (`'N'`, `'S'`, `'E'`, `'W'`) of a station id.
pub fn facility(id: &str) -> (Facility, Option<char>) {
    let b = id.as_bytes();
    let n = b.len();
    if n >= 4 && b[n - 3] == b'B' && matches!(b[n - 4], b'N' | b'S' | b'E' | b'W') {
        let dir = Some(b[n - 4] as char);
        match &id[n - 2..] {
            "SR" => return (Facility::Frontage, dir),
            "DR" => return (Facility::Ramp, dir),
            _ => {}
        }
    }
    (Facility::Main, None)
}

const COUNTIES: &str = include_str!("../reference/tx_counties.csv");

/// Normalized county name: letters only, upper case (`De Witt` = `DEWITT`).
fn county_key(s: &str) -> String {
    let s = s.trim().to_uppercase();
    let s = s.strip_suffix(" COUNTY").unwrap_or(&s);
    s.chars().filter(|c| c.is_ascii_alphabetic()).collect()
}

/// Texas county name (normalized) to 5-digit GEOID.
pub fn counties() -> HashMap<String, String> {
    csv::Reader::from_reader(COUNTIES.as_bytes())
        .records()
        .filter_map(|r| r.ok())
        .filter_map(|r| Some((county_key(r.get(1)?), r.get(0)?.to_string())))
        .collect()
}

/// A station snapped to the scenario road graph, as written to `counts.json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Count {
    pub station_id: String,
    pub year: u16,
    /// Annual average daily traffic, both directions.
    pub aadt: u32,
    pub on_road: String,
    pub lon: f64,
    pub lat: f64,
    /// Scenario link (node ids) the station sits on.
    pub from: u32,
    pub to: u32,
    /// Index of that link in `scenario.json` `road_links` (tells parallel
    /// links between the same nodes apart).
    pub link: u32,
    /// Opposite carriageway of a divided road, if found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from2: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to2: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link2: Option<u32>,
    /// TxDOT `ACTIVE`: the station is still in the count program.
    pub active: bool,
    /// Main lanes (two-way AADT), frontage road or ramp (one direction).
    pub facility: Facility,
    /// Direction of a frontage or ramp count: `N`, `S`, `E` or `W`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<char>,
    /// The station's county is outside the build area (no zones there).
    pub external: bool,
    /// Distance from the station to `from`-`to`, meters.
    pub dist_m: f64,
}

/// Reads `txdot_aadt.csv`; rows without coordinates or AADT are skipped.
pub fn read_stations(path: &Path) -> Result<Vec<Station>> {
    let mut r = csv::Reader::from_path(path).with_context(|| format!("open {}", path.display()))?;
    let h = r.headers()?.clone();
    let col = |n: &str| {
        h.iter()
            .position(|c| c == n)
            .with_context(|| format!("no column {n}"))
    };
    let (id, yr, q, road, lon, lat) = (
        col("TRFC_STATN_ID")?,
        col("AADT_RPT_YEAR")?,
        col("AADT_RPT_QTY")?,
        col("ON_ROAD")?,
        col("LONGITUDE")?,
        col("LATITUDE")?,
    );
    let act = h.iter().position(|c| c == "ACTIVE");
    let cnty = h.iter().position(|c| c == "CNTY_NM");
    let names = counties();
    let mut out = Vec::new();
    for rec in r.records() {
        let rec = rec?;
        let (Ok(lo), Ok(la), Ok(a)) = (
            rec[lon].parse::<f64>(),
            rec[lat].parse::<f64>(),
            rec[q].parse::<f64>(),
        ) else {
            continue;
        };
        if a <= 0.0 || lo == 0.0 || la == 0.0 {
            continue;
        }
        out.push(Station {
            id: rec[id].to_string(),
            year: rec[yr].parse::<f64>().map(|y| y as u16).unwrap_or(0),
            aadt: a as u32,
            on_road: rec[road].trim().to_string(),
            lon: lo,
            lat: la,
            active: act.and_then(|i| rec.get(i)).is_none_or(|v| v.trim() != "0"),
            county: cnty
                .and_then(|i| rec.get(i))
                .and_then(|v| names.get(&county_key(v)).cloned()),
        });
    }
    Ok(out)
}

/// A route number as (system, number), e.g. `("I", "35")`, `("FM", "2222")`.
fn route(token: &str) -> Option<(String, String)> {
    let t: String = token
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ')
        .collect();
    let t = t.trim();
    let split = t.find(|c: char| c.is_ascii_digit())?;
    let (p, n) = t.split_at(split);
    let p = p.split_whitespace().collect::<Vec<_>>().join(" ");
    let sys = match p.as_str() {
        "IH" | "I" | "INTERSTATE" => "I",
        "US" | "USHWY" | "US HWY" => "US",
        "SH" | "TX" | "STATE HIGHWAY" | "SR" => "SH",
        "FM" | "RM" | "RR" | "FARM TO MARKET" | "RANCH TO MARKET" | "FM RD" => "FM",
        "SL" | "LOOP" | "LP" => "LOOP",
        "SS" | "SPUR" | "SP" => "SPUR",
        "PR" | "PARK ROAD" => "PR",
        "BW" | "BELTWAY" => "BW",
        "BU" | "BS" | "BI" | "BUS" => "BUS",
        _ => return None,
    };
    let n = n.trim().trim_start_matches('0');
    let n: String = n
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    (!n.is_empty()).then(|| (sys.to_string(), n))
}

/// A street name as normalized words (`N LAMAR BOULEVARD` -> `N LAMAR BLVD`).
fn street(s: &str) -> Vec<String> {
    s.to_uppercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            match w {
                "BOULEVARD" => "BLVD",
                "STREET" => "ST",
                "AVENUE" => "AVE",
                "ROAD" => "RD",
                "DRIVE" => "DR",
                "PARKWAY" => "PKWY",
                "HIGHWAY" => "HWY",
                "LANE" => "LN",
                "FREEWAY" => "FWY",
                "EXPRESSWAY" => "EXPY",
                "NORTH" => "N",
                "SOUTH" => "S",
                "EAST" => "E",
                "WEST" => "W",
                x => x,
            }
            .to_string()
        })
        .collect()
}

/// True if a station's `ON_ROAD` names the OSM way with this `ref`/`name`.
pub fn matches(on_road: &str, osm_ref: Option<&str>, osm_name: Option<&str>) -> bool {
    if let Some(r) = route(on_road) {
        let refs = osm_ref.unwrap_or("");
        return refs
            .split([';', ',', '/'])
            .filter_map(route)
            .any(|x| x == r);
    }
    let a = street(on_road);
    let Some(b) = osm_name.map(street) else {
        return false;
    };
    if a.is_empty() || b.is_empty() {
        return false;
    }
    // Every word of the shorter name appears in the longer, in order.
    let (s, l) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    let mut it = l.iter();
    s.iter().all(|w| it.any(|x| x == w))
}

/// Meters per degree of longitude and latitude near `lat`.
fn scale(lat: f64) -> (f64, f64) {
    (111_320.0 * lat.to_radians().cos(), 110_574.0)
}

/// Distance from p to segment a-b (lon/lat) in meters, and the segment's
/// unit direction in local meters.
fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, [f64; 2]) {
    let (kx, ky) = scale(p[1]);
    let (ax, ay) = ((a[0] - p[0]) * kx, (a[1] - p[1]) * ky);
    let (bx, by) = ((b[0] - p[0]) * kx, (b[1] - p[1]) * ky);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (-(ax * dx + ay * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    let len = len2.sqrt().max(1e-9);
    ((cx * cx + cy * cy).sqrt(), [dx / len, dy / len])
}

const CELL: f64 = 0.01;

fn cell(lon: f64, lat: f64) -> (i32, i32) {
    ((lon / CELL).floor() as i32, (lat / CELL).floor() as i32)
}

/// Freeway main lanes and ramps: never a frontage-road count.
fn freeway(class: &str) -> bool {
    matches!(class, "motorway" | "motorway_link" | "trunk_link")
}

/// Unit compass vector (east, north) for a direction letter.
fn compass(d: char) -> [f64; 2] {
    match d {
        'N' => [0.0, 1.0],
        'S' => [0.0, -1.0],
        'E' => [1.0, 0.0],
        _ => [-1.0, 0.0],
    }
}

/// Snaps stations to scenario links. `edge_link[i]` is the scenario link of
/// `roads.edges[i]` (from `scenario::to_scenario_mapped`); `area` decides
/// `external`.
pub fn snap(
    stations: &[Station],
    roads: &Roads,
    edge_link: &[Option<crate::scenario::LinkRef>],
    area: &Area,
) -> Vec<Count> {
    // Segment index: (edge, a, b) by grid cell.
    let mut segs: Vec<(usize, [f64; 2], [f64; 2])> = Vec::new();
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, e) in roads.edges.iter().enumerate() {
        if edge_link[i].is_none() {
            continue;
        }
        let mut pts = vec![roads.nodes[e.from as usize]];
        pts.extend(e.shape.iter().copied());
        pts.push(roads.nodes[e.to as usize]);
        for w in pts.windows(2) {
            let k = segs.len();
            segs.push((i, w[0], w[1]));
            let (c0, c1) = (cell(w[0][0], w[0][1]), cell(w[1][0], w[1][1]));
            for x in c0.0.min(c1.0)..=c0.0.max(c1.0) {
                for y in c0.1.min(c1.1)..=c0.1.max(c1.1) {
                    grid.entry((x, y)).or_default().push(k);
                }
            }
        }
    }
    let mut out = Vec::new();
    for s in stations {
        let p = [s.lon, s.lat];
        let (cx, cy) = cell(s.lon, s.lat);
        let (fac, dirc) = facility(&s.id);
        // Best distance per matching edge, with its direction.
        let mut best: HashMap<usize, (f64, [f64; 2])> = HashMap::new();
        for x in cx - 1..=cx + 1 {
            for y in cy - 1..=cy + 1 {
                for &k in grid.get(&(x, y)).into_iter().flatten() {
                    let (ei, a, b) = segs[k];
                    let (d, dir) = seg_dist(p, a, b);
                    if d > MAX_M {
                        continue;
                    }
                    let e = &roads.edges[ei];
                    let named = matches(&s.on_road, e.r#ref.as_deref(), e.name.as_deref());
                    let ok = match fac {
                        Facility::Main => named,
                        Facility::Frontage => {
                            !freeway(&e.class)
                                && (named
                                    || e.name.as_deref().is_some_and(|n| {
                                        let n = n.to_lowercase();
                                        n.contains("frontage") || n.contains("service")
                                    }))
                        }
                        Facility::Ramp => e.class.ends_with("_link"),
                    };
                    // One-direction counts: a one-way edge must head that way.
                    let heading = dirc.is_none_or(|d| {
                        let c = compass(d);
                        !e.oneway || dir[0] * c[0] + dir[1] * c[1] > 0.0
                    });
                    if !ok || !heading {
                        continue;
                    }
                    let v = best.entry(ei).or_insert((d, dir));
                    if d < v.0 {
                        *v = (d, dir);
                    }
                }
            }
        }
        let mut cands: Vec<(usize, f64, [f64; 2])> =
            best.into_iter().map(|(e, (d, dir))| (e, d, dir)).collect();
        cands.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let Some(&(e0, d0, dir0)) = cands.first() else {
            continue;
        };
        let (from, to, link) = edge_link[e0].unwrap();
        let second = if fac == Facility::Main && roads.edges[e0].oneway {
            cands
                .iter()
                .skip(1)
                .find(|(e, _, dir)| {
                    roads.edges[*e].oneway
                        && dir[0] * dir0[0] + dir[1] * dir0[1] < -0.5
                        && edge_link[*e].map(|x| x.2) != Some(link)
                })
                .and_then(|(e, _, _)| edge_link[*e])
        } else {
            None
        };
        out.push(Count {
            station_id: s.id.clone(),
            year: s.year,
            aadt: s.aadt,
            on_road: s.on_road.clone(),
            lon: s.lon,
            lat: s.lat,
            from,
            to,
            link,
            from2: second.map(|x| x.0),
            to2: second.map(|x| x.1),
            link2: second.map(|x| x.2),
            active: s.active,
            facility: fac,
            direction: dirc,
            external: s.county.as_deref().is_some_and(|c| !area.contains(c)),
            dist_m: (d0 * 10.0).round() / 10.0,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RoadEdge;

    #[test]
    fn route_and_street_matching() {
        assert!(matches("IH0035", Some("I 35;US 290"), None));
        assert!(matches("US0290", Some("I 35;US 290"), None));
        assert!(matches("SH0071", Some("TX 71"), None));
        assert!(matches("FM2222", Some("RM 2222"), None));
        assert!(matches("SL0001", Some("Loop 1"), None));
        assert!(!matches("SH0071", Some("SH 710"), None));
        assert!(!matches("IH0035", None, Some("Interstate 35")));
        assert!(matches("LAMAR BLVD", None, Some("North Lamar Boulevard")));
        assert!(!matches("LAMAR BLVD", None, Some("Burnet Road")));
    }

    fn edge(from: u32, to: u32, oneway: bool, r: &str) -> RoadEdge {
        RoadEdge {
            from,
            to,
            length_m: 100.0,
            class: "motorway".into(),
            speed_kph: 100.0,
            lanes: 2,
            oneway,
            name: None,
            r#ref: Some(r.into()),
            shape: vec![],
        }
    }

    #[test]
    fn snaps_both_carriageways_and_drops_misses() {
        // Northbound 0->1 and southbound 3->2 about 30 m apart; a frontage
        // road 4->5 with another ref closer to the station.
        let roads = Roads {
            nodes: vec![
                [-97.7000, 30.30],
                [-97.7000, 30.31],
                [-97.6997, 30.30],
                [-97.6997, 30.31],
                [-97.69990, 30.30],
                [-97.69990, 30.31],
            ],
            edges: vec![
                edge(0, 1, true, "I 35"),
                edge(3, 2, true, "I 35"),
                edge(4, 5, false, "FM 1"),
            ],
        };
        let link = vec![Some((0, 1, 0)), Some((3, 2, 1)), Some((4, 5, 2))];
        let st = |id: &str, road: &str, lon: f64| Station {
            id: id.into(),
            year: 2023,
            aadt: 100_000,
            on_road: road.into(),
            lon,
            lat: 30.305,
            active: true,
            county: None,
        };
        let c = snap(
            &[
                st("a", "IH0035", -97.69995),
                st("far", "IH0035", -97.69),
                st("none", "SH0001", -97.6999),
            ],
            &roads,
            &link,
            &Area {
                prefixes: vec!["48453".into()],
            },
        );
        assert_eq!(c.len(), 1, "too far and unmatched stations dropped");
        assert_eq!((c[0].from, c[0].to), (0, 1));
        assert_eq!((c[0].from2, c[0].to2), (Some(3), Some(2)));
        assert_eq!((c[0].link, c[0].link2), (0, Some(1)));
        assert!(c[0].dist_m < 10.0);
        assert_eq!(c[0].facility, Facility::Main);
        assert!(!c[0].external);
    }

    #[test]
    fn facility_from_id_suffix() {
        assert_eq!(facility("106D1"), (Facility::Main, None));
        assert_eq!(facility("227H136SBSR"), (Facility::Frontage, Some('S')));
        assert_eq!(facility("227H136ANBSR"), (Facility::Frontage, Some('N')));
        assert_eq!(facility("102X9EBDR"), (Facility::Ramp, Some('E')));
        assert_eq!(facility("SR"), (Facility::Main, None));
    }

    #[test]
    fn county_names_map_to_geoids() {
        let c = counties();
        assert_eq!(c.len(), 254);
        assert_eq!(c[&county_key("TRAVIS")], "48453");
        assert_eq!(c[&county_key("De Witt")], "48123");
        assert_eq!(c[&county_key("Harris County")], "48201");
    }

    #[test]
    fn frontage_skips_main_lanes_and_county_sets_external() {
        // Freeway northbound 0->1, frontage road northbound 2->3 30 m east,
        // frontage southbound 5->4 30 m west (no ref, named Frontage Road).
        let mut fr_s = edge(5, 4, true, "x");
        fr_s.r#ref = None;
        fr_s.name = Some("Southbound Frontage Road".into());
        fr_s.class = "secondary".into();
        let mut fr_n = edge(2, 3, true, "Loop 1");
        fr_n.class = "secondary".into();
        let roads = Roads {
            nodes: vec![
                [-97.7000, 30.30],
                [-97.7000, 30.31],
                [-97.6997, 30.30],
                [-97.6997, 30.31],
                [-97.7003, 30.30],
                [-97.7003, 30.31],
            ],
            edges: vec![edge(0, 1, true, "Loop 1"), fr_n, fr_s],
        };
        let link = vec![Some((0, 1, 0)), Some((2, 3, 1)), Some((5, 4, 2))];
        let st = |id: &str, county: &str| Station {
            id: id.into(),
            year: 2025,
            aadt: 3_000,
            on_road: "SL0001".into(),
            lon: -97.7000,
            lat: 30.305,
            active: true,
            county: Some(county.into()),
        };
        let area = Area {
            prefixes: vec!["48453".into()],
        };
        let c = snap(
            &[
                st("main", "48453"),
                st("x1NBSR", "48453"),
                st("x2SBSR", "48491"),
                st("x3NBDR", "48453"),
            ],
            &roads,
            &link,
            &area,
        );
        assert_eq!(c.len(), 3, "ramp count with no ramp edge dropped");
        assert_eq!((c[0].link, c[0].facility), (0, Facility::Main));
        assert_eq!((c[1].link, c[1].facility), (1, Facility::Frontage));
        assert_eq!(c[1].link2, None, "one-direction counts take one link");
        assert_eq!((c[2].link, c[2].direction), (2, Some('S')));
        assert!(!c[1].external && c[2].external);
    }
}
