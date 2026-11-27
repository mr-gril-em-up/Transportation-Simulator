//! Input data schema. This is the contract between `data-pipeline` (producer)
//! and `sim-core` (consumer). Plain serde types, JSON on disk.
//!
//! Conventions:
//! - Coordinates are WGS84 `lon`, `lat` in degrees.
//! - Ids are `u32`, unique within their list. They do not need to be dense.
//! - Money is US dollars (2024), time is minutes unless a field says otherwise.
//! - Optional fields have defaults; the pipeline may omit them.

use serde::{Deserialize, Serialize};

/// Everything the simulation needs for one region (a county for M0/M1).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScenarioInput {
    /// Demand zones.
    pub zones: Vec<Zone>,
    /// Road graph nodes.
    pub road_nodes: Vec<RoadNode>,
    /// Road graph links.
    pub road_links: Vec<RoadLink>,
    /// Home-zone to work-zone worker flows (LODES, aggregated to zones).
    pub commutes: Vec<Commute>,
    /// Transit lines present at start (empty by default: the player builds them).
    #[serde(default)]
    pub lines: Vec<TransitLine>,
}

/// A demand zone (census block group for M0/M1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Zone {
    /// Unique zone id.
    pub id: u32,
    /// Centroid (population-weighted if available).
    pub lon: f64,
    /// Centroid latitude, degrees.
    pub lat: f64,
    /// Residents.
    pub population: f64,
    /// Jobs located in the zone (LODES WAC total).
    pub jobs: f64,
    /// Road node the zone's trips load onto (nearest drivable node to the centroid).
    pub road_node: u32,
    /// Share of households with no vehicle (ACS B08201). Default 0.06.
    #[serde(default = "default_zero_car_share")]
    pub zero_car_share: f64,
    /// Median household income in dollars (ACS B19013). Default 70,000.
    #[serde(default = "default_income")]
    pub median_income: f64,
    /// Daily parking cost at this zone as a destination. Default 0.
    #[serde(default)]
    pub parking_cost: f64,
    /// Land area in km². If omitted, estimated from spacing to the nearest zone.
    #[serde(default)]
    pub area_km2: Option<f64>,
}

/// A road graph node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoadNode {
    /// Unique node id.
    pub id: u32,
    /// Longitude, degrees.
    pub lon: f64,
    /// Latitude, degrees.
    pub lat: f64,
}

/// A drivable road segment between two nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoadLink {
    /// Start node id.
    pub from: u32,
    /// End node id.
    pub to: u32,
    /// Length, meters.
    pub length_m: f64,
    /// Free-flow speed.
    pub speed_kph: f64,
    /// Lanes per direction. Default 1.
    #[serde(default = "default_lanes")]
    pub lanes: u8,
    /// Capacity per direction, vehicles/hour. If omitted, derived from speed and lanes.
    #[serde(default)]
    pub capacity_vph: Option<f64>,
    /// Toll per traversal, dollars. Default 0.
    #[serde(default)]
    pub toll: f64,
    /// If false (default) the link is usable in both directions.
    #[serde(default)]
    pub oneway: bool,
    /// OSM highway class (`motorway`, `trunk`, `primary`, ..., `_link`
    /// variants). Optional; sets the derived capacity when `capacity_vph`
    /// is omitted (see `params::lane_capacity_vph`). F38.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

/// Workers living in `home` and working in `work` (zone ids).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commute {
    /// Home zone id.
    pub home: u32,
    /// Work zone id.
    pub work: u32,
    /// Workers making this commute (LODES S000).
    pub workers: f64,
}

/// Transit technologies. Every mode shares one model (stops, headway, speed,
/// fare); defaults per mode live in `params::ModeDefaults`.
/// Hyperloop is intentionally not a mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Diesel or electric bus in mixed traffic.
    Bus,
    /// Electric bus on overhead wire.
    Trolleybus,
    /// Bus rapid transit (dedicated lanes, stations).
    Brt,
    /// Street-running tram.
    Tram,
    /// Light rail, mostly segregated.
    LightRail,
    /// Commuter or regional rail.
    CommuterRail,
    /// Metro / subway (heavy rail).
    Metro,
    /// High-speed rail (300+ km/h).
    HighSpeedRail,
    /// Monorail.
    Monorail,
    /// Automated people mover.
    PeopleMover,
    /// Urban gondola (aerial cable car).
    Gondola,
    /// Maglev (500 km/h class).
    Maglev,
    /// Ferry or water taxi.
    Ferry,
}

/// A two-way transit line drawn by the player (or present at start).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitLine {
    /// Display name.
    #[serde(default)]
    pub name: String,
    /// Transit technology; sets the defaults.
    pub mode: Mode,
    /// Stops in order. At least two.
    pub stops: Vec<Stop>,
    /// Minutes between vehicles in each direction in the peak periods. Work
    /// trips see this headway. The fleet covers the more frequent of the two headways.
    pub headway_min: f64,
    /// Minutes between vehicles outside the peaks (midday, evening). Non-work
    /// trips see this headway. Default: same as `headway_min`.
    #[serde(default)]
    pub offpeak_headway_min: Option<f64>,
    /// Flat fare per boarding. Default from mode.
    #[serde(default)]
    pub fare: Option<f64>,
    /// Extra fare per km ridden (distance-based fares, e.g. HSR). Default from mode.
    #[serde(default)]
    pub fare_per_km: Option<f64>,
    /// Running speed between stops, excluding dwell. Default from mode.
    #[serde(default)]
    pub speed_kph: Option<f64>,
    /// Hours of service per day. Default from mode.
    #[serde(default)]
    pub span_hours: Option<f64>,
}

/// A stop on a line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stop {
    /// Longitude, degrees.
    pub lon: f64,
    /// Latitude, degrees.
    pub lat: f64,
    /// Display name.
    #[serde(default)]
    pub name: String,
    /// Has a park-and-ride lot (enables drive-to-transit access).
    #[serde(default)]
    pub park_ride: bool,
    /// Parking spaces this line may use at the lot (None: unlimited). When
    /// set, drive-to-transit riders above it are priced off (SIM-11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub park_ride_spaces: Option<f64>,
}

fn default_zero_car_share() -> f64 {
    0.06
}
fn default_income() -> f64 {
    70_000.0
}
fn default_lanes() -> u8 {
    1
}
