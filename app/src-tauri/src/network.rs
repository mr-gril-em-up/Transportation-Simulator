//! The "keep existing transit" setting (F44).
//!
//! The data pipeline writes today's transit network into
//! `ScenarioInput::lines`; sim-core treats those lines as existing service:
//! open on day 0, no capital charge, counted in the tax base. The shell keeps
//! them only when this setting asks for it, and clears them otherwise so the
//! game starts on a blank map. Off by default (APP-4): with placeholder bus
//! headways the loaded network costs about $670M a year to run against about
//! $233M of tax and fares, so it would start the player toward bankruptcy.
//! sim-core is fixing the cost side with real headways; the coordinator has
//! put the default starting state to Collins.

/// Set to `1`, `on` or `true` to start with the scenario's existing lines.
pub const NETWORK_ENV: &str = "TRANSPO_EXISTING_NETWORK";

/// Whether the setting asks for the existing network.
pub fn wanted() -> bool {
    matches!(
        std::env::var(NETWORK_ENV)
            .map(|v| v.to_ascii_lowercase())
            .as_deref(),
        Ok("1" | "on" | "true")
    )
}
