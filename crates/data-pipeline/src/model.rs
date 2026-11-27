//! Output types. These are the files the simulation core loads.
//!
//! Interim format: plain JSON, one file per layer. When sim-core publishes its
//! serde input schema, switch these to those types (or convert at write time).

use serde::{Deserialize, Serialize};

/// One demand zone (a 2020 census block group).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Zone {
    /// 12-digit block group GEOID: state(2) county(3) tract(6) block group(1).
    pub geoid: String,
    /// Population-weighted centroid (Census centers of population).
    pub lon: f64,
    pub lat: f64,
    /// 2020 decennial census population.
    pub population: u32,
    /// Workers living here (LODES RAC, all jobs).
    pub resident_workers: u32,
    /// Jobs located here (LODES WAC, all jobs).
    pub jobs: u32,
    /// Workers living here whose job is outside the study area.
    pub commute_out: u32,
    /// Jobs here filled by workers living outside the study area (in Texas).
    pub commute_in: u32,
    /// ACS 5-year median household income (B19013), dollars. Suppressed
    /// estimates take the tract, then county, household-weighted mean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_income: Option<f64>,
    /// Share of households with no vehicle (ACS B25044), same fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zero_car_share: Option<f64>,
    /// Land area in km² (TIGER/Line 2020 `ALAND`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area_km2: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Zones {
    pub zones: Vec<Zone>,
}

/// Home-to-work flow between two zones inside the study area.
/// `home` and `work` are indexes into `Zones::zones`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Flow {
    pub home: u32,
    pub work: u32,
    pub workers: u32,
}

/// Drivable road graph. Nodes are intersections and dead ends; each edge is a
/// way segment between two of them.
#[derive(Debug, Serialize, Deserialize)]
pub struct Roads {
    /// [lon, lat] per node.
    pub nodes: Vec<[f64; 2]>,
    pub edges: Vec<RoadEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoadEdge {
    pub from: u32,
    pub to: u32,
    pub length_m: f32,
    /// OSM highway class, e.g. "motorway", "residential".
    pub class: String,
    /// Posted speed if tagged, otherwise a default for the class.
    pub speed_kph: f32,
    /// Total lanes (both directions unless oneway); tagged or class default.
    pub lanes: u8,
    /// true: traversable from -> to only.
    pub oneway: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    /// OSM `ref` (route numbers such as `I 35;US 290`), for matching TxDOT
    /// count stations (`counts.rs`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub r#ref: Option<String>,
    /// Intermediate [lon, lat] points between `from` and `to`, for drawing.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub shape: Vec<[f64; 2]>,
}

/// Existing transit routes from OSM route relations. The game starts with
/// these removed by default; they are kept for calibration and the
/// "keep existing transit" toggle.
#[derive(Debug, Serialize, Deserialize)]
pub struct Transit {
    pub routes: Vec<TransitRoute>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TransitRoute {
    pub osm_id: i64,
    /// OSM route value: bus, trolleybus, tram, light_rail, train, subway, monorail.
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub r#ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub operator: Option<String>,
    /// Stops in route order.
    pub stops: Vec<Stop>,
    /// Weekday minutes between departures 07:00-09:00, from GTFS (`gtfs.rs`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub peak_headway_min: Option<f64>,
    /// Weekday minutes between departures 10:00-15:00, from GTFS.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub midday_headway_min: Option<f64>,
    /// Hours from first to last weekday departure, from GTFS.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub span_hours: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Stop {
    pub osm_id: i64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    pub lon: f64,
    pub lat: f64,
}
