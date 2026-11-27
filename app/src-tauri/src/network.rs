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

use sim_core::TransitLine;
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

#[cfg(test)]
mod tests {
    use super::*;

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
