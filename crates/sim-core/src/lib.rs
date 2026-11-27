//! Transpo-Sim simulation core: aggregate zone-to-zone demand, mode choice and
//! network assignment. See README.md for the input schema and public API.

pub mod schema;

pub use schema::*;

use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct SimError(pub String);

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SimError {}

fn err<T>(msg: impl Into<String>) -> Result<T, SimError> {
    Err(SimError(msg.into()))
}

impl ScenarioInput {
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
            if !(0.0..=1.0).contains(&z.zero_car_share) || z.population < 0.0 || z.jobs < 0.0 {
                return err(format!("zone {} has out-of-range values", z.id));
            }
        }
        for (i, l) in self.road_links.iter().enumerate() {
            if !nodes.contains(&l.from) || !nodes.contains(&l.to) {
                return err(format!("road link {i} references a missing node"));
            }
            if l.length_m <= 0.0 || l.speed_kph <= 0.0 {
                return err(format!("road link {i} needs positive length and speed"));
            }
        }
        for c in &self.commutes {
            if !zones.contains(&c.home) || !zones.contains(&c.work) {
                return err(format!("commute {}->{} references a missing zone", c.home, c.work));
            }
        }
        for l in &self.lines {
            l.validate()?;
        }
        Ok(())
    }
}

impl TransitLine {
    pub fn validate(&self) -> Result<(), SimError> {
        if self.stops.len() < 2 {
            return err(format!("line '{}' needs at least two stops", self.name));
        }
        if self.headway_min <= 0.0 {
            return err(format!("line '{}' needs a positive headway", self.name));
        }
        Ok(())
    }
}
