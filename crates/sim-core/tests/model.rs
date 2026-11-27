use sim_core::synth::{generate, straight_line, SynthConfig};
use sim_core::{Mode, ModeSplit, Params, Sim};

/// Lines open at once (`instant_build`), so ridership tests need not wait out construction.
fn sim() -> Sim {
    let p = Params {
        instant_build: true,
        ..Params::default()
    };
    Sim::new(generate(&SynthConfig::small()), p).unwrap()
}

/// Default params: lines build before opening, as in the game.
fn building_sim() -> Sim {
    Sim::new(generate(&SynthConfig::small()), Params::default()).unwrap()
}

fn rail() -> sim_core::TransitLine {
    straight_line(Mode::LightRail, (0.0, -7.0), (0.0, 7.0), 1.0, 8.0)
}

#[test]
fn baseline_is_car_dominated_with_conserved_trips() {
    let s = sim();
    let r = s.results();
    let sh = r.mode_share;
    assert!(sh.drive > 0.6, "drive share {sh:?}");
    assert_eq!(sh.transit(), 0.0);
    assert!((sh.total() - 1.0).abs() < 1e-9);
    // Every generated trip is either assigned a mode or counted as unserved.
    // Other trips: each zone's total trip budget minus its residents' work trips.
    let pres = s.params.commute_presence;
    let gen_other: f64 = s
        .input()
        .zones
        .iter()
        .map(|z| {
            let work: f64 = s
                .input()
                .commutes
                .iter()
                .filter(|c| c.home == z.id)
                .map(|c| c.workers * pres * 2.0)
                .sum();
            (z.population * s.params.trips_per_person - work).max(0.0)
        })
        .sum();
    let gen_work = s.input().commutes.iter().map(|c| c.workers).sum::<f64>() * pres * 2.0;
    let total = r.daily_trips.total() + r.unserved_trips;
    assert!(
        (total - gen_other - gen_work).abs() / total < 1e-6,
        "{total} vs {}",
        gen_other + gen_work
    );
    assert!(r.peak_avg_speed_kph > 10.0 && r.peak_avg_speed_kph < 110.0);
    assert!(s.link_flows().iter().any(|l| l.vc > 0.1));
}

#[test]
fn adding_a_line_creates_ridership_and_shifts_mode_share() {
    let mut s = sim();
    let drive_before = s.results().mode_share.drive;
    let id = s.add_line(rail()).unwrap();
    let r = s.results();
    assert!(r.car_times_stale);
    assert_eq!(r.lines.len(), 1);
    let l = &r.lines[0];
    assert_eq!(l.id, id);
    assert!(l.daily_boardings > 100.0, "boardings {}", l.daily_boardings);
    assert_eq!(l.boardings_by_stop.len(), 15);
    assert!((l.boardings_by_stop.iter().sum::<f64>() - l.daily_boardings).abs() < 1e-6);
    assert!(r.mode_share.transit() > 0.0);
    assert!(r.mode_share.drive < drive_before);
    assert!(l.operating_cost_per_day > 0.0 && l.fare_revenue_per_day > 0.0);
    // Daily load in each direction is roughly symmetric (PM returns mirror AM).
    let (f, b): (f64, f64) = l
        .segment_loads
        .iter()
        .fold((0.0, 0.0), |a, s| (a.0 + s[0], a.1 + s[1]));
    assert!(f > 0.0 && b > 0.0);
}

#[test]
fn fares_and_headways_move_ridership_the_right_way() {
    let mut s = sim();
    let id = s.add_line(rail()).unwrap();
    let base = s.results().lines[0].daily_boardings;

    let mut pricey = rail();
    pricey.fare = Some(6.0);
    s.update_line(id, pricey).unwrap();
    let after_fare = s.results().lines[0].daily_boardings;
    assert!(after_fare < base, "{after_fare} !< {base}");

    let mut rare = rail();
    rare.headway_min = 30.0;
    s.update_line(id, rare).unwrap();
    assert!(s.results().lines[0].daily_boardings < base);

    let mut fast = rail();
    fast.headway_min = 4.0;
    s.update_line(id, fast).unwrap();
    assert!(s.results().lines[0].daily_boardings > base);
}

#[test]
fn removing_a_line_restores_baseline_and_ids_stay_stable() {
    let mut s = sim();
    let base = s.results().mode_share;
    let a = s.add_line(rail()).unwrap();
    let b = s
        .add_line(straight_line(Mode::Bus, (-7.0, 1.0), (7.0, 1.0), 0.5, 10.0))
        .unwrap();
    assert_ne!(a, b);
    s.remove_line(a).unwrap();
    assert_eq!(s.results().lines.len(), 1);
    assert_eq!(s.results().lines[0].id, b);
    assert!(s.remove_line(a).is_err());
    s.remove_line(b).unwrap();
    let back = s.results().mode_share;
    assert!((back.drive - base.drive).abs() < 1e-9);
}

#[test]
fn transfers_between_crossing_lines_are_used() {
    let mut s = sim();
    s.add_line(rail()).unwrap();
    let cross = straight_line(Mode::Bus, (-7.0, 0.0), (7.0, 0.0), 1.0, 10.0);
    s.add_line(cross).unwrap();
    let total_boardings: f64 = s.results().lines.iter().map(|l| l.daily_boardings).sum();
    let transit_trips = s.results().daily_trips.transit();
    assert!(
        total_boardings > transit_trips,
        "expect some transfers: {total_boardings} vs {transit_trips}"
    );
}

#[test]
fn congestion_feedback_slows_peak_traffic() {
    let mut input = generate(&SynthConfig::small());
    let free = {
        let p = Params {
            bpr_alpha: 0.0,
            ..Default::default()
        };
        Sim::new(input.clone(), p)
            .unwrap()
            .results()
            .peak_avg_speed_kph
    };
    // Double the commuters.
    input.commutes.iter_mut().for_each(|c| c.workers *= 2.0);
    let congested = Sim::new(input, Params::default())
        .unwrap()
        .results()
        .peak_avg_speed_kph;
    assert!(congested < free, "{congested} !< {free}");
}

#[test]
fn line_searched_assignment_unjams_in_few_rounds() {
    // SIM-15: eight commuters for every real one jam the synthetic roads.
    // MSA's second round keeps half the free-flow load on the fastest
    // roads: 3.9 km/h after 2 rounds against 46 km/h settled. The line
    // search reaches 22 km/h in the same 2 rounds.
    let mut input = generate(&SynthConfig::small());
    input.commutes.iter_mut().for_each(|c| c.workers *= 8.0);
    let speed = |rounds| {
        let p = Params {
            feedback_iterations: rounds,
            ..Default::default()
        };
        Sim::new(input.clone(), p)
            .unwrap()
            .results()
            .peak_avg_speed_kph
    };
    let (two, settled) = (speed(2), speed(24));
    assert!(
        two > 0.3 * settled,
        "2 rounds {two:.1} km/h vs 24 rounds {settled:.1} km/h"
    );
}

#[test]
fn calibration_hits_targets() {
    let mut s = sim();
    let target = ModeSplit {
        drive: 0.75,
        carpool: 0.12,
        transit_walk: 0.0,
        transit_drive: 0.0,
        walk: 0.09,
        bike: 0.04,
    };
    s.calibrate_mode_shares(target, 15);
    let got = s.commute_mode_shares();
    for (g, t) in got.to_array().iter().zip(target.to_array()) {
        assert!((g - t).abs() < 0.01, "{got:?}");
    }
}

#[test]
fn calibration_fits_transit_as_one_share_or_by_access_mode() {
    let mut s = sim();
    s.add_line(rail()).unwrap();
    let mut target = ModeSplit {
        drive: 0.75,
        carpool: 0.10,
        transit_walk: 0.015,
        transit_drive: 0.0,
        walk: 0.06,
        bike: 0.04,
    };
    // Combined: total transit hits 1.5% (one synthetic line limits how many
    // commuters can ride at all), the access split is left to the model.
    s.calibrate_mode_shares(target, 20);
    let got = s.commute_mode_shares().shares();
    let want = target.shares();
    assert!((got.transit() - want.transit()).abs() < 0.002, "{got:?}");
    // Split: 0.5% park-and-ride, 1% walk access.
    target.transit_walk = 0.010;
    target.transit_drive = 0.005;
    let want = target.shares();
    s.calibrate_mode_shares(target, 20);
    let got = s.commute_mode_shares();
    assert!(
        (got.transit_walk - want.transit_walk).abs() < 0.002,
        "{got:?}"
    );
    assert!(
        (got.transit_drive - want.transit_drive).abs() < 0.002,
        "{got:?}"
    );
}

#[test]
fn tick_re_equilibrates_at_day_boundaries() {
    let mut s = sim();
    s.add_line(rail()).unwrap();
    assert!(s.results().car_times_stale);
    s.clock.set_timelapse(60.0);
    let days = s.tick(24.0); // one game day at 60x
    assert_eq!(days, 1);
    assert!(!s.results().car_times_stale);
}

#[test]
fn results_serialize_for_the_app() {
    let mut s = sim();
    s.add_line(rail()).unwrap();
    let json = serde_json::to_string(s.results()).unwrap();
    assert!(json.contains("boardings_by_stop"));
    assert!(serde_json::to_string(&s.zone_results()).is_ok());
    assert_eq!(s.desire_lines(20).len(), 20);
}

#[test]
fn tiny_fixture_runs() {
    let s = Sim::from_json(include_str!("fixtures/tiny.json"), Params::default()).unwrap();
    let r = s.results();
    assert_eq!(r.lines.len(), 1);
    assert!(r.daily_trips.total() > 0.0);
}

#[test]
fn sim_is_send_for_the_app_mutex() {
    fn assert_send<T: Send>() {}
    assert_send::<Sim>();
}

#[test]
fn warm_daily_rerun_stays_at_the_cold_equilibrium() {
    let mut s = sim();
    let cold = s.results().clone();
    s.equilibrate_warm();
    let warm = s.results();
    let rel = |a: f64, b: f64| (a - b).abs() / b.abs().max(1e-9);
    assert!(rel(warm.peak_avg_speed_kph, cold.peak_avg_speed_kph) < 0.03);
    assert!(rel(warm.mode_share.drive, cold.mode_share.drive) < 0.01);
    assert!(rel(warm.peak_vehicle_km, cold.peak_vehicle_km) < 0.03);
}

#[test]
fn preview_matches_build_and_charges_nothing() {
    let mut s = sim();
    assert!(!s.results().car_times_stale);
    let p = s.preview_line(rail()).unwrap();
    assert!(
        !s.results().car_times_stale,
        "preview must not mark car times stale"
    );
    assert_eq!(s.ledger().cash, 0.0);
    assert!(s.lines().is_empty());
    assert!(p.capital_cost > 0.0 && p.line.daily_boardings > 0.0);
    let id = s.add_line(rail()).unwrap();
    assert_eq!(id, 0, "preview must not use up a line id");
    let built = &s.results().lines[0];
    assert!((built.daily_boardings - p.line.daily_boardings).abs() < 1e-6);
    let l = s.ledger();
    assert!((l.current.capital_spent - p.capital_cost).abs() < 1e-6);
    assert!((l.current.federal_capital_grants - p.federal_grant).abs() < 1e-6);
    assert!((l.cash - (p.federal_grant - p.capital_cost)).abs() < 1e-6);
}

#[test]
fn extending_a_line_pays_only_the_difference() {
    let mut s = sim();
    let short = straight_line(Mode::Bus, (0.0, -3.0), (0.0, 3.0), 0.5, 10.0);
    let long = straight_line(Mode::Bus, (0.0, -7.0), (0.0, 7.0), 0.5, 10.0);
    let id = s.add_line(short.clone()).unwrap();
    let first = s.ledger().current.capital_spent;
    let full = s.preview_line(long.clone()).unwrap().capital_cost;
    s.update_line(id, long).unwrap();
    assert!((s.ledger().current.capital_spent - full).abs() < 1e-6);
    // Cutting it back refunds nothing.
    s.update_line(id, short).unwrap();
    assert!((s.ledger().current.capital_spent - full).abs() < 1e-6);
    assert!(first < full);
}

#[test]
fn month_settles_fares_costs_and_local_tax() {
    let mut s = sim();
    s.skip_to_next_month();
    let m = &s.ledger().history[0];
    assert_eq!(
        (m.local_tax, m.operating_cost),
        (0.0, 0.0),
        "no lines, no tax base"
    );
    s.add_line(rail()).unwrap();
    let before = s.ledger().cash;
    let capital = s.ledger().current.federal_capital_grants - s.ledger().current.capital_spent;
    s.skip_to_next_month();
    let l = s.ledger();
    assert_eq!(l.history.len(), 2);
    let m = &l.history[1];
    assert!(m.local_tax > 0.0 && l.served_population > 0.0);
    assert!(m.operating_cost > m.fare_revenue && m.fare_revenue > 0.0);
    assert!((l.cash - before - (m.net - capital)).abs() < 1e-3);
}

#[test]
fn new_lines_open_after_compressed_construction() {
    let mut s = building_sim();
    let opens = s.opening_day(Mode::LightRail);
    // 6 real years at 12x: about 6 game months.
    assert_eq!(opens, (6.0_f64 * 365.0 / 12.0).ceil() as u64);
    let p = s.preview_line(rail()).unwrap();
    let id = s.add_line(rail()).unwrap();
    assert!(s.results().lines.is_empty(), "no riders while building");
    assert_eq!(s.results().pending_lines[0].opens_day, opens);
    assert_eq!(s.lines().len(), 1);
    // Capital and grant are charged at the start, on the forecast.
    assert!((s.ledger().current.capital_spent - p.capital_cost).abs() < 1e-6);
    while s.clock.day() < opens {
        s.skip_to_next_month();
    }
    assert!(s.pending_lines().is_empty());
    let r = &s.results().lines[0];
    assert_eq!(r.id, id);
    assert!(r.daily_boardings > 0.0);
}

#[test]
fn service_changes_apply_now_and_realignments_wait() {
    let mut s = building_sim();
    s.params.instant_build = true;
    let id = s.add_line(rail()).unwrap();
    s.params.instant_build = false;
    let base = s.results().lines[0].daily_boardings;
    let paid = s.ledger().current.capital_spent;
    // Headway change: no construction, applies at once, extra vehicles are paid for.
    let mut faster = rail();
    faster.headway_min = 4.0;
    s.update_line(id, faster.clone()).unwrap();
    assert!(s.pending_lines().is_empty());
    assert!(
        s.ledger().current.capital_spent > paid,
        "more vehicles cost capital"
    );
    let frequent = s.results().lines[0].daily_boardings;
    assert!(frequent > base);
    // Extension: the old version keeps running until the new one opens.
    let mut longer = straight_line(Mode::LightRail, (0.0, -7.0), (0.0, 10.0), 1.0, 4.0);
    longer.name = faster.name.clone();
    s.update_line(id, longer).unwrap();
    let pend = s.pending_lines();
    assert!(pend[0].replaces_open_line);
    assert!((s.results().lines[0].daily_boardings - frequent).abs() < 1e-6);
    assert!(s.ledger().current.capital_spent > 0.0);
    // A service change during the rebuild reaches the running version too.
    let mut slower = s.lines()[0].1.clone();
    slower.headway_min = 15.0;
    s.update_line(id, slower).unwrap();
    assert!(s.results().lines[0].daily_boardings < frequent);
    assert_eq!(s.pending_lines()[0].opens_day, pend[0].opens_day);
    s.remove_line(id).unwrap();
    assert!(s.pending_lines().is_empty() && s.lines().is_empty());
}

#[test]
fn offpeak_headway_thins_non_work_riders_and_cost() {
    let mut s = sim();
    let id = s.add_line(rail()).unwrap();
    let all_day = s.results().lines[0].clone();
    // Same off-peak headway as peak: identical to leaving it unset.
    let mut same = rail();
    same.offpeak_headway_min = Some(same.headway_min);
    s.update_line(id, same).unwrap();
    let r = &s.results().lines[0];
    assert!((r.daily_boardings - all_day.daily_boardings).abs() < 1e-9);
    // Thinner midday service: fewer riders and fewer revenue hours, same fleet.
    let mut thin = rail();
    thin.offpeak_headway_min = Some(30.0);
    s.update_line(id, thin).unwrap();
    let r = &s.results().lines[0];
    assert!(r.daily_boardings < all_day.daily_boardings);
    assert!(r.revenue_hours_per_day < all_day.revenue_hours_per_day);
    assert_eq!(r.vehicles_required, all_day.vehicles_required);
    // Off-peak more frequent than peak: the fleet (and so capital) covers it.
    let mut busy_midday = rail();
    busy_midday.headway_min = 30.0;
    busy_midday.offpeak_headway_min = Some(4.0);
    let paid = s.ledger().current.capital_spent;
    s.update_line(id, busy_midday).unwrap();
    let r = &s.results().lines[0];
    assert!(r.vehicles_required > all_day.vehicles_required);
    assert!(s.ledger().current.capital_spent > paid);
}

#[test]
fn bond_pays_for_a_line_and_is_serviced_monthly() {
    let mut s = sim();
    s.add_line(rail()).unwrap();
    assert!(s.ledger().cash < 0.0, "capital left cash negative");
    // The synthetic county's tax capacity may not cover the whole line.
    let owed = (-s.ledger().cash).min(s.bond_capacity());
    let cash0 = s.ledger().cash;
    let bond = s.issue_bond(owed).unwrap();
    assert_eq!(bond.term_months, 360, "real 30-year term by default");
    assert!((s.ledger().cash - cash0 - owed).abs() < 1e-3);
    assert!(s.issue_bond(0.0).is_err() && s.issue_bond(f64::NAN).is_err());
    s.skip_to_next_month(); // issue month: no payment
    s.skip_to_next_month();
    let l = s.ledger();
    assert_eq!(l.history[0].debt_service, 0.0);
    let m = &l.history[1];
    assert!((m.debt_service - bond.payment).abs() < 1e-6);
    assert!(m.interest > 0.0 && m.interest < m.debt_service);
    assert!(l.debt() < owed);
}

#[test]
fn twelve_distressed_months_is_bankruptcy() {
    let mut s = sim();
    s.add_line(rail()).unwrap(); // capital unpaid: cash below zero
    for _ in 0..11 {
        s.skip_to_next_month();
    }
    assert_eq!(s.ledger().bankrupt_month, None);
    s.skip_to_next_month();
    let l = s.ledger();
    assert_eq!(l.distressed_months, 12);
    assert_eq!(l.bankrupt_month, Some(l.history.last().unwrap().month));
    assert!(s.issue_bond(1e6).is_err(), "no borrowing once bankrupt");
}

#[test]
fn borrowing_is_capped_by_tax_capacity() {
    let mut s = sim();
    assert_eq!(s.bond_capacity(), 0.0, "no lines, no tax area, no pledge");
    // A line under construction already counts toward the pledged tax area.
    let mut b = building_sim();
    b.add_line(rail()).unwrap();
    assert!(b.bond_capacity() > 0.0 && b.ledger().served_population == 0.0);
    s.add_line(rail()).unwrap();
    let cap = s.bond_capacity();
    assert!(cap > 0.0 && s.tax_capacity() > 0.0);
    assert!(s.issue_bond(cap * 1.01).is_err(), "above capacity");
    s.issue_bond(cap * 0.5).unwrap();
    let left = s.bond_capacity();
    assert!((left - cap * 0.5).abs() < cap * 1e-6, "half used");
    s.issue_bond(left).unwrap();
    assert!(s.bond_capacity() < cap * 1e-6);
    assert!(s.issue_bond(cap * 0.01).is_err(), "capacity used up");
}

#[test]
fn bond_for_a_line_under_construction_waits_for_opening_before_coverage() {
    let mut s = building_sim();
    s.add_line(rail()).unwrap();
    let opens = s.pending_lines()[0].opens_day;
    let amount = s.bond_capacity().min(-s.ledger().cash).max(1.0);
    let b = s.issue_bond(amount).unwrap();
    let opens_month = opens.div_ceil(30);
    assert_eq!(b.coverage_from_month, b.issued_month + 1 + opens_month);
}

#[test]
fn bus_segments_follow_the_roads_and_rail_does_not() {
    let mut s = sim();
    // Stops on grid corners (1 km grid): each diagonal hop is 2 km of street.
    let bus = straight_line(Mode::Bus, (-4.0, -4.0), (4.0, 4.0), 2f64.sqrt(), 10.0);
    let id = s.add_line(bus.clone()).unwrap();
    let r = s.results().lines.iter().find(|l| l.id == id).unwrap();
    assert_eq!(r.segment_shapes.len(), bus.stops.len() - 1);
    for (k, shape) in r.segment_shapes.iter().enumerate() {
        let (a, b) = (&bus.stops[k], &bus.stops[k + 1]);
        assert!(shape.len() >= 3, "segment {k} routed via a corner");
        assert_eq!(shape[0], [a.lon, a.lat]);
        assert_eq!(*shape.last().unwrap(), [b.lon, b.lat]);
    }
    let per_seg = r.length_km / r.segment_shapes.len() as f64;
    assert!((per_seg - 2.0).abs() < 0.02, "street km {per_seg}");

    let rail = straight_line(Mode::LightRail, (-4.0, -4.0), (4.0, 4.0), 2f64.sqrt(), 10.0);
    let id = s.add_line(rail).unwrap();
    let r = s.results().lines.iter().find(|l| l.id == id).unwrap();
    assert!(r.segment_shapes.is_empty(), "rail keeps alignment_factor");
}

#[test]
fn bus_stop_off_the_road_network_falls_back_to_a_straight_segment() {
    let mut s = sim();
    // The synthetic region spans +-8 km; the last stop is 20 km out.
    let bus = straight_line(Mode::Bus, (0.0, 2.0), (0.0, 20.0), 6.0, 10.0);
    let id = s.add_line(bus).unwrap();
    let r = s.results().lines.iter().find(|l| l.id == id).unwrap();
    assert!(!r.segment_shapes[0].is_empty(), "inside the grid: routed");
    assert!(
        r.segment_shapes.last().unwrap().is_empty(),
        "off-network: straight"
    );
}

#[test]
fn full_park_and_ride_lots_price_drivers_out() {
    // Unlimited lots first: how many cars each end terminal draws.
    let mut s = sim();
    let id = s.add_line(rail()).unwrap();
    let free = s.results().lines[0].park_ride_cars_by_stop.clone();
    let last = free.len() - 1;
    assert!(free[0] > 20.0 && free[last] > 20.0, "{free:?}");
    assert!(free[1..last].iter().all(|&c| c == 0.0), "{free:?}");
    let free_riders = s.results().lines[0].daily_boardings;

    // Lots a third that size: the shadow price fills them, no more.
    let mut line = rail();
    let spaces = [free[0] / 3.0, free[last] / 3.0];
    line.stops[0].park_ride_spaces = Some(spaces[0]);
    line.stops[last].park_ride_spaces = Some(spaces[1]);
    s.remove_line(id).unwrap();
    s.add_line(line).unwrap();
    let l = &s.results().lines[0];
    for (cars, cap) in [l.park_ride_cars_by_stop[0], l.park_ride_cars_by_stop[last]]
        .into_iter()
        .zip(spaces)
    {
        assert!(
            cars <= cap * 1.1 && cars >= cap * 0.5,
            "{cars} vs {cap} spaces"
        );
    }
    assert!(l.daily_boardings < free_riders);
}

fn cutoff_sim(cut: Option<f64>) -> Sim {
    let p = Params {
        instant_build: true,
        strategic_max_min: cut,
        ..Params::default()
    };
    let mut s = Sim::new(generate(&SynthConfig::small()), p).unwrap();
    s.add_line(rail()).unwrap();
    s
}

#[test]
fn strategic_cutoff_covering_every_pair_matches_the_dense_model() {
    let (dense, cut) = (cutoff_sim(None), cutoff_sim(Some(1e6)));
    let (a, b) = (dense.results(), cut.results());
    assert_eq!(b.beyond_cutoff_trips, 0.0);
    for (x, y) in a
        .daily_trips
        .to_array()
        .iter()
        .zip(b.daily_trips.to_array())
    {
        assert!((x - y).abs() < 1e-6 * x.max(1.0), "{a:?} vs {b:?}");
    }
    let (la, lb) = (&a.lines[0], &b.lines[0]);
    assert!((la.daily_boardings - lb.daily_boardings).abs() < 1e-6 * la.daily_boardings);
}

#[test]
fn short_strategic_cutoff_drops_far_pairs_but_conserves_trips() {
    let dense = cutoff_sim(None);
    let s = cutoff_sim(Some(8.0));
    let r = s.results();
    assert!(r.beyond_cutoff_trips > 0.0);
    // Every generated trip is modeled, unserved or beyond the cutoff.
    let total =
        |r: &sim_core::Results| r.daily_trips.total() + r.unserved_trips + r.beyond_cutoff_trips;
    let (t0, t1) = (total(dense.results()), total(r));
    assert!((t0 - t1).abs() / t0 < 1e-6, "{t0} vs {t1}");
    // Non-work trips are redistributed within the cutoff, not lost.
    assert!(r.other_trips.total() > 0.99 * dense.results().other_trips.total());
    assert!(r.lines[0].daily_boardings > 0.0);
}

#[test]
fn every_kept_pair_under_a_cutoff_has_a_car_time() {
    // One-way links make road times asymmetric; a kept pair must still be
    // drivable both ways, or it silently loses its car option.
    let mut input = generate(&SynthConfig::small());
    let mut slow = Vec::new();
    for (n, l) in input.road_links.iter_mut().enumerate() {
        if n % 3 == 0 {
            l.oneway = true;
            slow.push(sim_core::RoadLink {
                from: l.to,
                to: l.from,
                speed_kph: l.speed_kph / 3.0,
                ..l.clone()
            });
        }
    }
    input.road_links.extend(slow);
    let p = Params {
        strategic_max_min: Some(8.0),
        ..Params::default()
    };
    let dense = Sim::new(input.clone(), Params::default()).unwrap();
    let s = Sim::new(input, p).unwrap();
    assert_eq!(dense.car_unreachable_pairs(), 0);
    assert_eq!(s.car_unreachable_pairs(), 0);
}
