use sim_core::{Mode, ScenarioInput};

fn tiny() -> ScenarioInput {
    ScenarioInput::from_json(include_str!("fixtures/tiny.json")).unwrap()
}

#[test]
fn fixture_parses_and_validates_with_defaults() {
    let s = tiny();
    s.validate().unwrap();
    assert_eq!(s.zones[0].zero_car_share, 0.06);
    assert_eq!(s.zones[0].median_income, 70_000.0);
    assert_eq!(s.road_links[0].toll, 0.0);
    assert!(!s.road_links[0].oneway);
    assert_eq!(s.lines[0].mode, Mode::CommuterRail);
    assert!(s.lines[0].stops[1].park_ride);
}

#[test]
fn json_round_trip() {
    let s = tiny();
    let back = ScenarioInput::from_json(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back.zones.len(), 3);
    assert_eq!(back.road_links[2].capacity_vph, Some(900.0));
}

#[test]
fn validate_catches_dangling_references() {
    let mut s = tiny();
    s.commutes[0].work = 99;
    assert!(s.validate().is_err());
    let mut s = tiny();
    s.zones[0].road_node = 99;
    assert!(s.validate().is_err());
    let mut s = tiny();
    s.lines[0].stops.pop();
    assert!(s.validate().is_err());
}

#[test]
fn validate_rejects_nan_zero_lanes_and_self_loops() {
    let mut s = tiny();
    s.road_links[0].lanes = 0;
    assert!(s.validate().is_err());
    s.road_links[0].capacity_vph = Some(800.0);
    s.validate().unwrap(); // explicit capacity makes 0 lanes acceptable
    let mut s = tiny();
    s.road_links[0].speed_kph = f64::NAN;
    assert!(s.validate().is_err());
    let mut s = tiny();
    s.road_links[0].to = s.road_links[0].from;
    assert!(s.validate().is_err());
    let mut s = tiny();
    s.lines[0].headway_min = f64::NAN;
    assert!(s.validate().is_err());
}
