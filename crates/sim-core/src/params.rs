//! Model parameters and per-mode defaults. All numbers are starting points for
//! calibration, chosen from typical US values (see comments).

use crate::schema::Mode;
use serde::{Deserialize, Serialize};

/// Person-level travel options the logit model chooses between.
pub const N_CHOICES: usize = 6;
/// Index of drive alone.
pub const DRIVE: usize = 0;
/// Index of carpool.
pub const CARPOOL: usize = 1;
/// Index of transit with walk access.
pub const TRANSIT_WALK: usize = 2;
/// Index of transit with drive (park-and-ride) access.
pub const TRANSIT_DRIVE: usize = 3;
/// Index of walk.
pub const WALK: usize = 4;
/// Index of bike.
pub const BIKE: usize = 5;

/// Every tunable number in the model. See docs/MODEL.md for what each does and where defaults come from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Params {
    /// Logit coefficient per generalized minute (negative).
    pub beta_per_min: f64,
    /// Alternative-specific constants, indexed DRIVE..BIKE. Drive is the reference (0).
    pub asc: [f64; N_CHOICES],
    /// Out-of-vehicle time weights relative to in-vehicle time.
    pub wait_weight: f64,
    /// Weight on walk minutes (access, egress, transfers) relative to in-vehicle time.
    pub walk_weight: f64,
    /// Penalty per transfer, minutes.
    pub transfer_penalty_min: f64,
    /// Early-arrival penalty weight for transit work trips (schedule delay).
    pub schedule_delay_weight: f64,

    /// Value of time as a share of the hourly wage (USDOT uses ~50% for personal travel).
    pub vot_wage_share: f64,
    /// Floor on value of time, $/hour.
    pub vot_min_per_hour: f64,
    /// VOT used to rank transit paths, $/hour (USDOT local personal ~ $18).
    pub path_vot_per_hour: f64,

    /// Perceived car operating cost, $/km (fuel + wear, ~$0.20/mile).
    pub car_cost_per_km: f64,
    /// Average persons per carpool vehicle.
    pub carpool_occupancy: f64,
    /// Extra minutes for picking up or dropping off.
    pub carpool_extra_min: f64,
    /// Park and walk at each end of a car trip, minutes.
    pub car_terminal_min: f64,
    /// Share of the daily parking cost charged per work trip / other trip.
    pub parking_share_work: f64,
    /// Share of the daily parking cost charged per non-work trip.
    pub parking_share_other: f64,

    /// Walking speed, km/h.
    pub walk_kph: f64,
    /// Cycling speed, km/h.
    pub bike_kph: f64,
    /// Longest trip anyone walks, km (network distance).
    pub max_walk_km: f64,
    /// Longest trip anyone bikes, km (network distance).
    pub max_bike_km: f64,
    /// Network distance / straight-line distance for walk, bike and access legs.
    pub detour_factor: f64,
    /// Straight-line walk radius to a stop.
    pub stop_walk_km: f64,
    /// Max walk between stops of different lines for a transfer.
    pub transfer_walk_km: f64,
    /// Longest drive to a park-and-ride, minutes.
    pub pnr_max_drive_min: f64,
    /// Minutes to park and walk to the platform.
    pub pnr_park_min: f64,

    /// Share of workers commuting on a given day (absence + work from home).
    pub commute_presence: f64,
    /// Total person trips per resident per day, all purposes (2022 NHTS: 2.28).
    /// Non-work trips are this total minus the zone's work trips, floored at 0.
    pub trips_per_person: f64,
    /// Gravity model decay per car minute for non-work trips.
    pub gravity_decay_per_min: f64,
    /// Desired arrival time at work: shares per 15-minute bin from `arrival_profile_start_h`.
    pub work_arrival_profile: Vec<f64>,
    /// Hour of day (decimal) the first profile bin starts.
    pub arrival_profile_start_h: f64,
    /// Share of daily non-work trips in the AM peak hour.
    pub other_peak_hour_share: f64,

    /// BPR curve alpha (time = t0 x (1 + alpha x (v/c)^beta)).
    pub bpr_alpha: f64,
    /// BPR curve beta.
    pub bpr_beta: f64,
    /// Outer iterations of mode choice <-> car assignment (method of successive averages).
    pub feedback_iterations: usize,
    /// Rounds for the daily warm-started re-run (`Sim::equilibrate_warm`).
    pub daily_iterations: usize,

    /// Build and permitting times are divided by this (12x default, 1x-24x setting).
    pub construction_speedup: f64,
    /// Open lines the moment they are drawn (no construction period). Off by default.
    pub instant_build: bool,

    /// Transit authority cash at the start, dollars. 0: the first line is
    /// paid from grants and future tax (bonds arrive with F26).
    pub starting_cash: f64,
    /// Share of a line's capital cost the federal government pays when the
    /// line passes the cost-effectiveness gate (FTA Capital Investment Grants: up to 80%, typically ~50%).
    pub federal_capital_share: f64,
    /// Gate: highest capital cost per forecast daily boarding that still gets a federal grant, dollars.
    pub federal_max_capital_per_daily_rider: f64,
    /// Federal and state operating assistance as a share of operating cost.
    pub operating_assistance_share: f64,
    /// Local transit sales tax rate (CapMetro, DART, Houston METRO: 1%).
    pub local_sales_tax_rate: f64,
    /// Taxable retail sales per resident per year, dollars.
    pub taxable_sales_per_capita: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            beta_per_min: -0.035,
            asc: [0.0, -1.6, -1.2, -2.2, -0.8, -2.6],
            wait_weight: 2.0,
            walk_weight: 2.0,
            transfer_penalty_min: 5.0,
            schedule_delay_weight: 0.5,
            vot_wage_share: 0.5,
            vot_min_per_hour: 8.0,
            path_vot_per_hour: 18.0,
            car_cost_per_km: 0.125,
            carpool_occupancy: 2.3,
            carpool_extra_min: 5.0,
            car_terminal_min: 3.0,
            parking_share_work: 0.5,
            parking_share_other: 0.25,
            walk_kph: 4.8,
            bike_kph: 15.0,
            max_walk_km: 4.0,
            max_bike_km: 15.0,
            detour_factor: 1.3,
            stop_walk_km: 0.8,
            transfer_walk_km: 0.4,
            pnr_max_drive_min: 25.0,
            pnr_park_min: 3.0,
            commute_presence: 0.8,
            trips_per_person: 2.28,
            gravity_decay_per_min: 0.08,
            // 5:00 to 10:00 in 15-minute bins, shaped like ACS "time arriving at work".
            work_arrival_profile: vec![
                1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 7.5, 8.0, 7.5, 7.0, 6.0, 5.0, 4.5, 3.5,
                2.5, 2.0, 1.5, 1.0,
            ],
            arrival_profile_start_h: 5.0,
            other_peak_hour_share: 0.07,
            bpr_alpha: 0.15,
            bpr_beta: 4.0,
            feedback_iterations: 4,
            daily_iterations: 1,
            construction_speedup: 12.0,
            instant_build: false,
            starting_cash: 0.0,
            federal_capital_share: 0.5,
            // Austin Light Rail Phase 1 (in the CIG pipeline): ~$7.1B for 18,700
            // forecast riders is ~$380k per rider; 600k leaves room above it. Estimate.
            federal_max_capital_per_daily_rider: 600_000.0,
            // figures.csv `operating_assistance_share`: CapMetro NTD 2023, federal
            // $50.9M + state $0 over $356.3M. Upper bound (pandemic relief).
            operating_assistance_share: 0.143,
            // figures.csv `transit_sales_tax_rate`: CapMetro, DART, METRO (MTA cap).
            local_sales_tax_rate: 0.01,
            // figures.csv `taxable_sales_per_capita`: CapMetro service area, 2023$.
            taxable_sales_per_capita: 20_600.0,
        }
    }
}

impl Params {
    /// Share of work trips whose desired arrival falls in the busiest hour.
    pub fn work_peak_hour_share(&self) -> f64 {
        let p = &self.work_arrival_profile;
        let total: f64 = p.iter().sum();
        if total <= 0.0 {
            return 0.0;
        }
        let best = p
            .windows(4.min(p.len()))
            .map(|w| w.iter().sum::<f64>())
            .fold(0.0, f64::max);
        best / total
    }

    /// Hour of day (decimal) when the busiest arrival hour starts.
    pub fn work_peak_hour_start(&self) -> f64 {
        let p = &self.work_arrival_profile;
        let n = 4.min(p.len());
        let (idx, _) = p
            .windows(n)
            .map(|w| w.iter().sum::<f64>())
            .enumerate()
            .fold((0, f64::MIN), |a, (i, s)| if s > a.1 { (i, s) } else { a });
        self.arrival_profile_start_h + idx as f64 * 0.25
    }

    /// Expected wait for a headway: half the headway when frequent; riders time
    /// their arrival for infrequent service, so long headways cost less than h/2.
    pub fn wait_min(&self, headway: f64) -> f64 {
        if headway <= 12.0 {
            headway / 2.0
        } else {
            6.0 + (headway - 12.0) * 0.25
        }
    }

    /// In-game build duration for a real-world duration.
    pub fn build_days(&self, real_days: f64) -> f64 {
        real_days / self.construction_speedup.max(1.0)
    }
}

/// Per-mode defaults. Costs are approximate 2020s US ranges (overview doc);
/// projected or foreign-only modes use systems abroad.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ModeDefaults {
    /// Running speed between stops.
    pub speed_kph: f64,
    /// Dwell per intermediate stop, seconds.
    pub dwell_s: f64,
    /// Route distance / straight-line distance between stops.
    pub alignment_factor: f64,
    /// Passengers per vehicle (or train).
    pub vehicle_capacity: f64,
    /// Operating cost per car revenue hour, dollars (NTD counts rail revenue
    /// hours per car, so this is the number NTD publishes).
    pub cost_per_car_hour: f64,
    /// Cars per vehicle (train). 1 for buses and single-unit modes.
    pub cars_per_vehicle: f64,
    /// Default flat fare per boarding, dollars.
    pub fare: f64,
    /// Default distance fare, dollars per km.
    pub fare_per_km: f64,
    /// Default hours of service per day.
    pub span_hours: f64,
    /// Capital cost of guideway and stations per route km, dollars.
    pub capital_per_km: f64,
    /// Purchase price per car, dollars.
    pub vehicle_cost: f64,
    /// Real-world planning and construction time, years (divided by `construction_speedup` in game).
    pub build_years: f64,
}

impl ModeDefaults {
    /// Operating cost per vehicle (train) revenue hour: per-car cost x cars.
    pub fn cost_per_revenue_hour(&self) -> f64 {
        self.cost_per_car_hour * self.cars_per_vehicle
    }
}

/// Default speeds, capacities, costs and fares for a mode.
pub fn mode_defaults(m: Mode) -> ModeDefaults {
    // Operating columns: speed, dwell, alignment, capacity, cost per car hour,
    // cars, fare, fare per km, span.
    let d = |speed_kph,
             dwell_s,
             alignment_factor,
             vehicle_capacity,
             cost,
             cars,
             fare,
             fare_per_km,
             span_hours| {
        ModeDefaults {
            speed_kph,
            dwell_s,
            alignment_factor,
            vehicle_capacity,
            cost_per_car_hour: cost,
            cars_per_vehicle: cars,
            fare,
            fare_per_km,
            span_hours,
            capital_per_km: 0.0,
            vehicle_cost: 0.0,
            build_years: 0.0,
        }
    };
    // Bus, light rail and commuter rail cost per car hour and the CapMetro
    // fares come from data-pipeline/reference/figures.csv (FTA NTD 2023,
    // CapMetro), checked by tests/figures.rs. Other modes are estimates.
    let ops = match m {
        Mode::Bus => d(28.0, 20.0, 1.25, 70.0, 182.0, 1.0, 1.25, 0.0, 18.0),
        Mode::Trolleybus => d(28.0, 20.0, 1.25, 70.0, 190.0, 1.0, 1.25, 0.0, 18.0),
        Mode::Brt => d(38.0, 25.0, 1.15, 100.0, 200.0, 1.0, 1.25, 0.0, 19.0),
        Mode::Tram => d(32.0, 25.0, 1.15, 160.0, 300.0, 1.0, 1.25, 0.0, 19.0),
        Mode::LightRail => d(55.0, 30.0, 1.1, 300.0, 436.0, 2.0, 1.25, 0.0, 20.0),
        Mode::CommuterRail => d(80.0, 45.0, 1.1, 400.0, 716.0, 2.0, 3.5, 0.0, 16.0),
        Mode::Metro => d(70.0, 30.0, 1.1, 1000.0, 1200.0, 1.0, 2.0, 0.0, 20.0),
        Mode::HighSpeedRail => d(270.0, 120.0, 1.05, 400.0, 3000.0, 1.0, 20.0, 0.15, 18.0),
        Mode::Monorail => d(50.0, 30.0, 1.05, 300.0, 450.0, 1.0, 1.5, 0.0, 18.0),
        Mode::PeopleMover => d(40.0, 20.0, 1.0, 100.0, 400.0, 1.0, 0.0, 0.0, 20.0),
        Mode::Gondola => d(20.0, 0.0, 1.0, 10.0, 250.0, 1.0, 1.25, 0.0, 16.0),
        Mode::Maglev => d(400.0, 120.0, 1.0, 600.0, 4000.0, 1.0, 30.0, 0.2, 18.0),
        Mode::Ferry => d(30.0, 120.0, 1.1, 150.0, 600.0, 1.0, 2.0, 0.0, 16.0),
    };
    // Capital: guideway per km and price per car. Sourced rows (figures.csv,
    // per mile / 1.609): bus and electric bus prices (MTC), LRV price, Austin
    // light rail $840M/mile, US mostly-surface rail $117M/mile (tram), mostly-
    // tunnel $570M/mile (metro), Texas Central HSR $167M/mile. Others are estimates.
    let (capital_per_km, vehicle_cost) = match m {
        Mode::Bus => (0.3e6, 716_400.0),
        Mode::Trolleybus => (3.0e6, 1_407_600.0),
        Mode::Brt => (15.0e6, 1_407_600.0),
        Mode::Tram => (117.0e6 / MI_KM, 5.0e6),
        Mode::LightRail => (840.0e6 / MI_KM, 5.0e6),
        Mode::CommuterRail => (20.0e6, 5.0e6),
        Mode::Metro => (570.0e6 / MI_KM, 15.0e6),
        Mode::HighSpeedRail => (167.0e6 / MI_KM, 40.0e6),
        Mode::Monorail => (150.0e6, 10.0e6),
        Mode::PeopleMover => (100.0e6, 5.0e6),
        Mode::Gondola => (30.0e6, 0.1e6),
        Mode::Maglev => (300.0e6, 60.0e6),
        Mode::Ferry => (0.0, 10.0e6),
    };
    // Planning plus construction, real years. Light rail 6: the roadmap's F24
    // check, and Austin Phase 1 (FTA engineering 2027, service 2033). Others
    // are estimates from typical US and foreign projects.
    let build_years = match m {
        Mode::Bus => 0.5,
        Mode::Trolleybus => 1.5,
        Mode::Brt => 2.5,
        Mode::Tram => 4.0,
        Mode::LightRail => 6.0,
        Mode::CommuterRail => 4.0,
        Mode::Metro => 9.0,
        Mode::HighSpeedRail => 10.0,
        Mode::Monorail => 5.0,
        Mode::PeopleMover => 4.0,
        Mode::Gondola => 2.0,
        Mode::Maglev => 10.0,
        Mode::Ferry => 1.0,
    };
    ModeDefaults {
        capital_per_km,
        vehicle_cost,
        build_years,
        ..ops
    }
}

/// Kilometers per mile, for converting the per-mile cost figures.
pub const MI_KM: f64 = 1.609_344;

/// Default road capacity per lane by free-flow speed (HCM-style rough values).
pub fn lane_capacity_vph(speed_kph: f64) -> f64 {
    if speed_kph >= 80.0 {
        1900.0
    } else if speed_kph >= 55.0 {
        900.0
    } else {
        600.0
    }
}
