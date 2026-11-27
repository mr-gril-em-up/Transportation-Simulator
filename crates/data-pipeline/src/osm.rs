//! Road and transit networks from an OpenStreetMap .osm.pbf extract.
//!
//! Two passes over the file: ways and relations first (to learn which nodes
//! matter), then nodes (coordinates and stop names for just those). Works on
//! the Geofabrik Texas extract directly; `bbox` clips it to the study area.

use crate::model::{RoadEdge, Roads, Stop, Transit, TransitRoute};
use anyhow::Result;
use osmpbf::{Element, ElementReader, RelMemberType};
use std::collections::HashMap;
use std::path::Path;

/// [min_lon, min_lat, max_lon, max_lat]
pub type BBox = [f64; 4];

fn in_bbox(b: &BBox, lon: f64, lat: f64) -> bool {
    lon >= b[0] && lon <= b[2] && lat >= b[1] && lat <= b[3]
}

/// Drivable highway classes, with default speed (km/h) and default total lanes.
/// Service roads, tracks and paths are left out to keep the graph small.
fn class_defaults(class: &str) -> Option<(f32, u8)> {
    Some(match class {
        "motorway" => (105.0, 4),
        "trunk" => (90.0, 4),
        "primary" => (72.0, 4),
        "secondary" => (64.0, 2),
        "tertiary" => (56.0, 2),
        "motorway_link" | "trunk_link" => (72.0, 1),
        "primary_link" | "secondary_link" | "tertiary_link" => (48.0, 1),
        "unclassified" => (48.0, 2),
        "residential" => (40.0, 2),
        "living_street" => (16.0, 1),
        _ => return None,
    })
}

/// Classes kept by `--roads highway` (DATA-8): motorway through secondary and
/// their ramps, for the statewide strategic tier (F34).
pub fn is_highway_class(class: &str) -> bool {
    let base = class.strip_suffix("_link").unwrap_or(class);
    matches!(base, "motorway" | "trunk" | "primary" | "secondary")
}

const TRANSIT_MODES: &[&str] = &[
    "bus",
    "trolleybus",
    "tram",
    "light_rail",
    "train",
    "subway",
    "monorail",
];

/// "65 mph" -> 104.6, "50" -> 50 (OSM default unit is km/h).
/// Parses OSM `maxspeed` to km/h. Anything that is not a plausible speed
/// (`0`, `nan`, `inf`, typos above 200 km/h) returns `None` so the class
/// default applies; sim-core rejects links with non-positive speed.
fn parse_speed(s: &str) -> Option<f32> {
    let s = s.trim();
    let v = match s.strip_suffix("mph") {
        Some(n) => n.trim().parse::<f32>().ok()? * 1.609_344,
        None => s.parse::<f32>().ok()?,
    };
    (v.is_finite() && v > 0.0 && v <= 200.0).then_some(v)
}

fn haversine_m(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (la1, la2) = (a[1].to_radians(), b[1].to_radians());
    let dla = la2 - la1;
    let dlo = (b[0] - a[0]).to_radians();
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * 6_371_008.8 * h.sqrt().asin()
}

struct WayRec {
    refs: Vec<i64>,
    class: String,
    speed_kph: f32,
    lanes: u8,
    oneway: bool,
    reversed: bool,
    name: Option<String>,
}

struct RouteRec {
    osm_id: i64,
    mode: String,
    r#ref: Option<String>,
    name: Option<String>,
    operator: Option<String>,
    stop_ids: Vec<i64>,
}

/// With `highway_only`, roads below secondary are skipped (see
/// [`is_highway_class`]); transit is read either way.
pub fn read_osm(path: &Path, bbox: &BBox, highway_only: bool) -> Result<(Roads, Transit)> {
    // Pass 1: ways and route relations.
    let mut ways: Vec<WayRec> = Vec::new();
    let mut routes: Vec<RouteRec> = Vec::new();
    let mut node_uses: HashMap<i64, u32> = HashMap::new();
    ElementReader::from_path(path)?.for_each(|el| match el {
        Element::Way(w) => {
            let tags: HashMap<&str, &str> = w.tags().collect();
            let Some(class) = tags.get("highway") else {
                return;
            };
            let Some((def_speed, def_lanes)) = class_defaults(class) else {
                return;
            };
            if highway_only && !is_highway_class(class) {
                return;
            }
            if tags.get("area") == Some(&"yes") {
                return;
            }
            let ow = tags.get("oneway").copied().unwrap_or("");
            let reversed = ow == "-1";
            let oneway = reversed
                || matches!(ow, "yes" | "true" | "1")
                || (ow != "no"
                    && (class.ends_with("motorway")
                        || tags.get("junction") == Some(&"roundabout")));
            let lanes = tags
                .get("lanes")
                .and_then(|l| l.split(';').next()?.trim().parse::<u8>().ok())
                .unwrap_or(if oneway {
                    def_lanes.div_ceil(2).max(1)
                } else {
                    def_lanes
                });
            let speed_kph = tags
                .get("maxspeed")
                .and_then(|s| parse_speed(s))
                .unwrap_or(def_speed);
            let refs: Vec<i64> = w.refs().collect();
            if refs.len() < 2 {
                return;
            }
            for (i, r) in refs.iter().enumerate() {
                // Endpoints count twice so they always become graph nodes.
                let bump = if i == 0 || i == refs.len() - 1 { 2 } else { 1 };
                *node_uses.entry(*r).or_default() += bump;
            }
            ways.push(WayRec {
                refs,
                class: class.to_string(),
                speed_kph,
                lanes,
                oneway,
                reversed,
                name: tags.get("name").map(|s| s.to_string()),
            });
        }
        Element::Relation(r) => {
            let tags: HashMap<&str, &str> = r.tags().collect();
            if tags.get("type") != Some(&"route") {
                return;
            }
            let Some(mode) = tags.get("route").filter(|m| TRANSIT_MODES.contains(m)) else {
                return;
            };
            let nodes: Vec<(String, i64)> = r
                .members()
                .filter(|m| m.member_type == RelMemberType::Node)
                .map(|m| (m.role().unwrap_or("").to_string(), m.member_id))
                .collect();
            // Prefer stop positions; fall back to platforms when a route only maps those.
            let mut stop_ids: Vec<i64> = nodes
                .iter()
                .filter(|(role, _)| role.starts_with("stop"))
                .map(|n| n.1)
                .collect();
            if stop_ids.is_empty() {
                stop_ids = nodes
                    .iter()
                    .filter(|(role, _)| role.starts_with("platform"))
                    .map(|n| n.1)
                    .collect();
            }
            if stop_ids.is_empty() {
                return;
            }
            routes.push(RouteRec {
                osm_id: r.id(),
                mode: mode.to_string(),
                r#ref: tags.get("ref").map(|s| s.to_string()),
                name: tags.get("name").map(|s| s.to_string()),
                operator: tags.get("operator").map(|s| s.to_string()),
                stop_ids,
            });
        }
        _ => {}
    })?;

    // Pass 2: coordinates for road nodes and stops, names for stops.
    let mut coords: HashMap<i64, [f64; 2]> = HashMap::with_capacity(node_uses.len());
    let mut stop_names: HashMap<i64, Option<String>> = routes
        .iter()
        .flat_map(|r| r.stop_ids.iter().map(|&id| (id, None)))
        .collect();
    let mut keep = |id: i64, lon: f64, lat: f64, name: Option<&str>| {
        let is_stop = stop_names.contains_key(&id);
        if is_stop || node_uses.contains_key(&id) {
            coords.insert(id, [lon, lat]);
        }
        if is_stop {
            stop_names.insert(id, name.map(str::to_string));
        }
    };
    ElementReader::from_path(path)?.for_each(|el| match el {
        Element::Node(n) => keep(
            n.id(),
            n.lon(),
            n.lat(),
            n.tags().find(|t| t.0 == "name").map(|t| t.1),
        ),
        Element::DenseNode(n) => keep(
            n.id(),
            n.lon(),
            n.lat(),
            n.tags().find(|t| t.0 == "name").map(|t| t.1),
        ),
        _ => {}
    })?;

    Ok((
        build_roads(&ways, &node_uses, &coords, bbox),
        build_transit(routes, &coords, &stop_names, bbox),
    ))
}

/// Splits ways at shared nodes. A segment is kept only if every point is
/// inside the bbox, so the clipped graph has no dangling half-edges.
fn build_roads(
    ways: &[WayRec],
    uses: &HashMap<i64, u32>,
    coords: &HashMap<i64, [f64; 2]>,
    bbox: &BBox,
) -> Roads {
    let mut node_idx: HashMap<i64, u32> = HashMap::new();
    let mut nodes: Vec<[f64; 2]> = Vec::new();
    let mut edges = Vec::new();
    let mut intern = |id: i64, c: [f64; 2], nodes: &mut Vec<[f64; 2]>| {
        *node_idx.entry(id).or_insert_with(|| {
            nodes.push(c);
            (nodes.len() - 1) as u32
        })
    };
    for w in ways {
        let pts: Vec<Option<[f64; 2]>> = w.refs.iter().map(|r| coords.get(r).copied()).collect();
        let mut start = 0;
        for i in 1..w.refs.len() {
            if uses.get(&w.refs[i]).copied().unwrap_or(0) < 2 {
                continue;
            }
            let seg = &pts[start..=i];
            if seg
                .iter()
                .all(|p| p.is_some_and(|p| in_bbox(bbox, p[0], p[1])))
            {
                let seg: Vec<[f64; 2]> = seg.iter().map(|p| p.unwrap()).collect();
                let length_m = seg.windows(2).map(|p| haversine_m(p[0], p[1])).sum::<f64>() as f32;
                let (a, b) = (
                    intern(w.refs[start], seg[0], &mut nodes),
                    intern(w.refs[i], seg[seg.len() - 1], &mut nodes),
                );
                let mut shape: Vec<[f64; 2]> = seg[1..seg.len() - 1].to_vec();
                let (from, to) = if w.reversed {
                    shape.reverse();
                    (b, a)
                } else {
                    (a, b)
                };
                if from != to || length_m > 0.0 {
                    edges.push(RoadEdge {
                        from,
                        to,
                        length_m,
                        class: w.class.clone(),
                        speed_kph: w.speed_kph,
                        lanes: w.lanes,
                        oneway: w.oneway,
                        name: w.name.clone(),
                        shape,
                    });
                }
            }
            start = i;
        }
    }
    Roads { nodes, edges }
}

fn build_transit(
    routes: Vec<RouteRec>,
    coords: &HashMap<i64, [f64; 2]>,
    names: &HashMap<i64, Option<String>>,
    bbox: &BBox,
) -> Transit {
    let mut out: Vec<TransitRoute> = routes
        .into_iter()
        .filter_map(|r| {
            let stops: Vec<Stop> = r
                .stop_ids
                .iter()
                .filter_map(|id| {
                    let c = coords.get(id)?;
                    Some(Stop {
                        osm_id: *id,
                        name: names.get(id).cloned().flatten(),
                        lon: c[0],
                        lat: c[1],
                    })
                })
                .collect();
            // Keep routes that serve the study area; keep their full stop list.
            stops
                .iter()
                .any(|s| in_bbox(bbox, s.lon, s.lat))
                .then_some(TransitRoute {
                    osm_id: r.osm_id,
                    mode: r.mode,
                    r#ref: r.r#ref,
                    name: r.name,
                    operator: r.operator,
                    peak_headway_min: None,
                    midday_headway_min: None,
                    span_hours: None,
                    stops,
                })
        })
        .collect();
    out.sort_by_key(|r| r.osm_id);
    Transit { routes: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds() {
        assert!((parse_speed("65 mph").unwrap() - 104.6).abs() < 0.1);
        assert_eq!(parse_speed("50"), Some(50.0));
        assert_eq!(parse_speed("none"), None);
        for bad in ["0", "0 mph", "-30", "nan", "inf", "550"] {
            assert_eq!(parse_speed(bad), None, "{bad}");
        }
    }

    #[test]
    fn distance() {
        // Texas Capitol to UT Tower, about 1.27 km north.
        let d = haversine_m([-97.7404, 30.2747], [-97.7394, 30.2861]);
        assert!((d - 1270.0).abs() < 60.0, "{d}");
    }
}
