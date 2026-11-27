//! The "start with today's network" setting (F44, APP-4).
//!
//! The data pipeline writes today's CapMetro network as sim-core
//! `TransitLine`s to `existing_lines.json` beside `scenario.json` (DATA-7),
//! and into `scenario.json` `lines` only when built with `--existing-lines`.
//! sim-core treats `ScenarioInput::lines` as existing service: open on day 0,
//! no capital charge, stops in the tax base. With the setting on (the default
//! while Collins decides the starting state) the shell uses the scenario's
//! own lines, or else `existing_lines.json`; with it off it clears them and
//! the game starts on a blank map.
//!
//! It also reads the real rail and intercity lines sim-core does not run yet
//! (`reference`), which the map draws in the statewide view.

use serde::{Deserialize, Serialize};
use sim_core::TransitLine;
use std::collections::HashMap;
use std::path::Path;

/// Set to `0`, `off` or `false` to start on a blank map.
pub const NETWORK_ENV: &str = "TRANSPO_EXISTING_NETWORK";

/// Whether the setting asks for today's network (anything but 0/off/false).
pub fn wanted() -> bool {
    !matches!(
        std::env::var(NETWORK_ENV)
            .map(|v| v.to_ascii_lowercase())
            .as_deref(),
        Ok("0" | "off" | "false")
    )
}

/// The lines in `existing_lines.json` beside `scenario`, or none if the file
/// is missing (older builds). A malformed file is an error, not a blank map.
pub fn load(scenario: &Path) -> Result<Vec<TransitLine>, String> {
    let path = scenario.with_file_name("existing_lines.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let json = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&json).map_err(|e| format!("{}: {e}", path.display()))
}

/// A real rail or intercity line from the pipeline's `transit.json` (OSM
/// route relations) that sim-core does not run: drawn on the map so the
/// statewide view shows today's regional network, never scored.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct RefLine {
    /// `r<osm relation id>` of the variant kept.
    pub id: String,
    /// Display name, e.g. "Trinity Railway Express" or "DART Red Line".
    pub name: String,
    pub operator: String,
    /// OSM route type: `train`, `light_rail` or `tram`.
    pub mode: String,
    /// OSM `ref` (e.g. "RED", "TRE"), for the client's line colors.
    pub r#ref: String,
    pub stops: Vec<RefStop>,
}

/// One stop of a [`RefLine`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RefStop {
    #[serde(default)]
    pub name: String,
    pub lon: f64,
    pub lat: f64,
}

#[derive(Deserialize)]
struct TransitFile {
    routes: Vec<OsmRoute>,
}

#[derive(Deserialize)]
struct OsmRoute {
    osm_id: u64,
    mode: String,
    #[serde(default)]
    r#ref: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    operator: Option<String>,
    stops: Vec<RefStop>,
}

/// Rail modes drawn as reference lines. Buses are too many to draw statewide;
/// monorail in Texas is airport people movers.
const REF_MODES: [&str; 3] = ["train", "light_rail", "tram"];

/// The real rail and intercity lines in `transit.json` beside `scenario`, one
/// per (operator, line): OSM keeps a relation per direction and branch, and
/// the variant with the most stops stands for the line. CapMetro's are left
/// out because they run in sim-core as today's network. A missing file gives
/// none; a malformed one is an error.
pub fn reference(scenario: &Path) -> Result<Vec<RefLine>, String> {
    let path = scenario.with_file_name("transit.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let json = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file: TransitFile =
        serde_json::from_str(&json).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(pick_reference(file.routes))
}

fn pick_reference(routes: Vec<OsmRoute>) -> Vec<RefLine> {
    let mut best: HashMap<(String, String), OsmRoute> = HashMap::new();
    for r in routes {
        let operator = r.operator.clone().unwrap_or_default();
        if !REF_MODES.contains(&r.mode.as_str())
            || r.stops.len() < 2
            || operator.starts_with("Capital Metro")
        {
            continue;
        }
        let key = (operator, line_name(&r));
        match best.get(&key) {
            Some(b) if b.stops.len() >= r.stops.len() => {}
            _ => {
                best.insert(key, r);
            }
        }
    }
    let mut out: Vec<RefLine> = best
        .into_iter()
        .map(|((operator, name), r)| RefLine {
            id: format!("r{}", r.osm_id),
            name,
            operator,
            mode: r.mode,
            r#ref: r.r#ref.unwrap_or_default(),
            stops: r.stops,
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// "Red Line: Westmoreland => Parker Road" by DART -> "DART Red Line": the
/// name before the direction, prefixed with DART where it would clash with
/// Houston's METRORail colors.
fn line_name(r: &OsmRoute) -> String {
    let base = r
        .name
        .as_deref()
        .map(|n| n.split(':').next().unwrap_or(n).trim().to_string())
        .filter(|n| !n.is_empty())
        .or_else(|| r.r#ref.clone())
        .unwrap_or_else(|| format!("Line {}", r.osm_id));
    let dart = r.operator.as_deref() == Some("Dallas Area Rapid Transit");
    if dart && base.ends_with(" Line") && !base.contains("DART") {
        format!("DART {base}")
    } else {
        base
    }
}

#[derive(Deserialize)]
struct ZonesFile {
    zones: Vec<ZoneGeo>,
}

#[derive(Deserialize)]
struct ZoneGeo {
    geoid: String,
    lon: f64,
    lat: f64,
}

/// Every zone's centroid and county FIPS (`[lon, lat, fips]`, the first five
/// GEOID digits) from `zones.json` beside `scenario`, so the client can name
/// the county under the camera. None if the file is missing (synthetic).
pub fn zone_counties(scenario: &Path) -> Result<Vec<[f64; 3]>, String> {
    let path = scenario.with_file_name("zones.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let json = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file: ZonesFile =
        serde_json::from_str(&json).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(file
        .zones
        .iter()
        .filter_map(|z| Some([z.lon, z.lat, z.geoid.get(..5)?.parse().ok()?]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_counties_reads_fips_from_geoid() {
        let dir = std::env::temp_dir().join(format!("transpo-zc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let scenario = dir.join("scenario.json");
        assert!(zone_counties(&scenario).unwrap().is_empty());
        std::fs::write(
            dir.join("zones.json"),
            r#"{"zones":[{"geoid":"48453001100","lon":-97.74,"lat":30.27,"population":1}]}"#,
        )
        .unwrap();
        assert_eq!(
            zone_counties(&scenario).unwrap(),
            [[-97.74, 30.27, 48453.0]]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reference_keeps_one_rail_variant_per_line() {
        let routes: TransitFile = serde_json::from_str(
            r#"{"routes":[
            {"osm_id":1,"mode":"light_rail","ref":"RED","name":"Red Line: A => B","operator":"Dallas Area Rapid Transit",
             "stops":[{"name":"A","lon":-96.8,"lat":32.7},{"name":"B","lon":-96.7,"lat":32.9}]},
            {"osm_id":2,"mode":"light_rail","ref":"RED","name":"Red Line: B => A","operator":"Dallas Area Rapid Transit",
             "stops":[{"name":"B","lon":-96.7,"lat":32.9},{"name":"M","lon":-96.75,"lat":32.8},{"name":"A","lon":-96.8,"lat":32.7}]},
            {"osm_id":3,"mode":"train","ref":"550","operator":"Capital Metropolitan Transportation Authority",
             "stops":[{"lon":-97.7,"lat":30.2},{"lon":-97.8,"lat":30.6}]},
            {"osm_id":4,"mode":"bus","ref":"1","stops":[{"lon":-97.7,"lat":30.2},{"lon":-97.8,"lat":30.6}]},
            {"osm_id":5,"mode":"train","ref":"Texas Eagle","name":"Amtrak Texas Eagle: Chicago => San Antonio","operator":"Amtrak",
             "stops":[{"lon":-97.1,"lat":32.7},{"lon":-98.5,"lat":29.4}]}
            ]}"#,
        )
        .unwrap();
        let out = pick_reference(routes.routes);
        let names: Vec<&str> = out.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            names,
            ["Amtrak Texas Eagle", "DART Red Line"],
            "no CapMetro, no bus"
        );
        assert_eq!(out[1].id, "r2", "the variant with the most stops");
        assert_eq!(out[1].stops.len(), 3);
    }

    #[test]
    fn reads_existing_lines_beside_the_scenario() {
        let dir = std::env::temp_dir().join(format!("transpo-net-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let scenario = dir.join("scenario.json");
        assert!(load(&scenario).unwrap().is_empty(), "no file: blank map");
        std::fs::write(
            dir.join("existing_lines.json"),
            r#"[{"name":"CapMetro 801","mode":"bus","headway_min":10,
            "stops":[{"lon":-97.74,"lat":30.27},{"lon":-97.72,"lat":30.30}]}]"#,
        )
        .unwrap();
        let lines = load(&scenario).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].name, "CapMetro 801");
        std::fs::write(dir.join("existing_lines.json"), "{").unwrap();
        assert!(load(&scenario).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
