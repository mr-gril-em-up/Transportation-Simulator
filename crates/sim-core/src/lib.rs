//! Transpo-Sim simulation core: aggregate zone-to-zone demand, mode choice and
//! network assignment. See README.md for the public API and docs/ for the model.

#![warn(missing_docs)]

pub mod clock;
pub mod economy;
mod geo;
mod graph;
mod intercity;
pub mod model;
mod pairs;
pub mod params;
mod roads;
pub mod schema;
pub mod synth;
mod transit;

pub use clock::Clock;
pub use economy::{Bond, Ledger, MonthStatement};
pub use model::{
    DesireLine, IntercityPair, LineId, LinePreview, LineResult, LinkFlow, ModeSplit, PendingLine,
    Results, RoadState, Sim, ZoneResult,
};
pub use params::{IntercityParams, Params};
pub use schema::*;
use std::collections::HashSet;
use std::fmt;

/// Error returned by validation and `Sim` calls; the string is a readable message.
#[derive(Debug, Clone, PartialEq)]
pub struct SimError(pub String);

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SimError {}

// These return false for NaN, so `!positive(x)` rejects NaN too.
fn positive(x: f64) -> bool {
    x > 0.0
}
fn non_negative(x: f64) -> bool {
    x >= 0.0
}

fn err<T>(msg: impl Into<String>) -> Result<T, SimError> {
    Err(SimError(msg.into()))
}

impl ScenarioInput {
    /// Parse a scenario from JSON. Does not validate; call `validate` (or `Sim::new`, which does).
    pub fn from_json(s: &str) -> Result<Self, SimError> {
        serde_json::from_str(s).map_err(|e| SimError(format!("bad scenario json: {e}")))
    }

    /// Check referential integrity. Called by the simulation before use; the
    /// data pipeline should call it on its output too.
    pub fn validate(&self) -> Result<(), SimError> {
        let nodes: HashSet<u32> = self.road_nodes.iter().map(|n| n.id).collect();
        if nodes.len() != self.road_nodes.len() {
            return err("duplicate road node id");
        }
        let zones: HashSet<u32> = self.zones.iter().map(|z| z.id).collect();
        if zones.len() != self.zones.len() {
            return err("duplicate zone id");
        }
        if self.zones.is_empty() {
            return err("no zones");
        }
        for z in &self.zones {
            if !nodes.contains(&z.road_node) {
                return err(format!("zone {} road_node {} missing", z.id, z.road_node));
            }
            let ok = (0.0..=1.0).contains(&z.zero_car_share)
                && z.population >= 0.0
                && z.jobs >= 0.0
                && z.median_income >= 0.0
                && z.parking_cost >= 0.0
                && z.lon.is_finite()
                && z.lat.is_finite()
                && z.area_km2.is_none_or(|a| a > 0.0);
            if !ok {
                return err(format!("zone {} has out-of-range values", z.id));
            }
        }
        for (i, l) in self.road_links.iter().enumerate() {
            if !nodes.contains(&l.from) || !nodes.contains(&l.to) {
                return err(format!("road link {i} references a missing node"));
            }
            if !positive(l.length_m) || !positive(l.speed_kph) {
                return err(format!("road link {i} needs positive length and speed"));
            }
            if l.from == l.to {
                return err(format!("road link {i} is a self-loop"));
            }
            if l.lanes == 0 && l.capacity_vph.is_none() {
                return err(format!("road link {i} has 0 lanes and no capacity_vph"));
            }
            if l.capacity_vph.is_some_and(|c| !positive(c)) || !non_negative(l.toll) {
                return err(format!("road link {i} has invalid capacity or toll"));
            }
        }
        for c in &self.commutes {
            if !non_negative(c.workers) {
                return err(format!(
                    "commute {}->{} has invalid workers",
                    c.home, c.work
                ));
            }
            if !zones.contains(&c.home) || !zones.contains(&c.work) {
                return err(format!(
                    "commute {}->{} references a missing zone",
                    c.home, c.work
                ));
            }
        }
        for l in &self.lines {
            l.validate()?;
        }
        Ok(())
    }
}

impl TransitLine {
    /// Check the line has two or more stops, a positive headway, and non-negative fares, speed and span.
    pub fn validate(&self) -> Result<(), SimError> {
        if self.stops.len() < 2 {
            return err(format!("line '{}' needs at least two stops", self.name));
        }
        if !positive(self.headway_min) || !self.offpeak_headway_min.is_none_or(positive) {
            return err(format!("line '{}' needs a positive headway", self.name));
        }
        let pos = |v: Option<f64>| v.is_none_or(|x| x >= 0.0);
        if !pos(self.fare) || !pos(self.fare_per_km) || !pos(self.span_hours) {
            return err(format!("line '{}' has a negative fare or span", self.name));
        }
        if self.speed_kph.is_some_and(|s| !positive(s)) {
            return err(format!("line '{}' needs a positive speed", self.name));
        }
        if self
            .stops
            .iter()
            .any(|s| !s.lon.is_finite() || !s.lat.is_finite())
        {
            return err(format!(
                "line '{}' has a non-finite stop coordinate",
                self.name
            ));
        }
        Ok(())
    }
}
