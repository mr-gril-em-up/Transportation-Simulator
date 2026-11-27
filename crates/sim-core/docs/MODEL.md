# sim-core model

This is the math the simulation runs, in the order it runs, with the reason for
every simplification. Parameter names refer to fields of `Params`
([src/params.rs](../src/params.rs)); defaults are shown in brackets.

The design goal (Game Overview doc): "real enough that a planner nods, cheap
enough to re-run for a metro in under 2 seconds." Everything here is a standard
four-step travel model (generation, distribution, mode choice, assignment) cut
down to what moves the player's numbers, with a feedback loop between mode
choice and road congestion. Full activity-based models take hours per run and
are out of scope.

## 1. Zones

A zone is a census block group for M0/M1 (Travis County has roughly 700). Each
zone has a centroid, residents, jobs, zero-car share, median income, parking
cost and an optional area.

- **Zone radius** `r = sqrt(area / pi)`. Without an area, `r = 0.56 x` distance
  to the nearest other centroid (0.56 is the equal-area disk radius of a square
  cell). Used for stop coverage and intrazonal distance.
- **Intrazonal distance** `0.9 r` (the mean distance between two random points
  in a disk is about 0.9 r), minimum 0.3 km.
- **Value of time** per zone: `vot_wage_share [0.5] x median_income / 2080`
  dollars per hour, floor `vot_min_per_hour [$8]`. USDOT guidance values local
  personal travel at about 50% of the wage; $70k income gives about $17/hour,
  close to the doc's $18 baseline.

Why zones instead of a synthetic population: the doc's rule is that the
aggregate tier is the truth and agents are only the close-up. Zone flows
are what the score uses, and they are far cheaper. The synthetic population is
an M2 concern (local-tier agents sampled from these flows).

## 2. Trip generation

Two purposes:

| Purpose | Source | Daily trips |
|---|---|---|
| Work (home-based work) | `commutes` (LODES home zone to work zone) | `workers x commute_presence [0.8]` each way, so x2 per day |
| Other (shopping, errands, social, school) | `max(0, population x trips_per_person [2.28] - the zone's daily work trips)` | one-way trips from each zone |

- `commute_presence` covers absence and work-from-home. Austin's WFH share is
  among the highest in the US (roughly 20%+), so 0.8 is the starting point.
- `trips_per_person` is the **total** daily trip rate, all purposes: 2.28 from
  the 2022 NHTS (FHWA Summary of Travel Trends, Table 2-9), row
  `trips_per_person_day_2022` in
  [figures.csv](../../data-pipeline/reference/figures.csv). Collins chose the
  sourced figure over the design doc's ~3.5 (the 2017 value was 3.37). The
  report warns 2022 is post-pandemic and may not be a steady state, so this is
  the number to revisit in F13.
- Work trips are subtracted per home zone, so a zone of commuters makes fewer
  other trips and the total stays 2.28 x population. A zone whose work trips
  exceed its budget (possible with LODES vs census mismatch) makes 0 other
  trips rather than a negative number, so the region total can sit slightly
  above 2.28 x population.

**Why work trips come straight from LODES and not a model:** LODES is observed
home-to-work geography (Round Rock to the Domain, Katy to the Energy Corridor).
Modeling it would only add error.

## 3. Destination choice for other trips (gravity model)

For origin `i`, trips to `j` are proportional to `A_j x exp(-gravity_decay_per_min [0.08] x t_ij)`,
where `A_j = jobs_j + 0.5 x population_j` and `t_ij` is free-flow car time.
Rows are scaled so each origin produces exactly its trips (singly
constrained).

Simplifications and why:
- Attraction uses jobs plus half of residents as a stand-in for POIs, shops and
  visits. The pipeline can later supply a proper attraction field (OSM POIs,
  LODES retail jobs) without code changes beyond the formula.
- Destinations use car time only, not a mode-choice logsum. That keeps
  destinations fixed when a line is added, so a line edit never has to rerun
  distribution. Cost: a great transit line does not yet pull trips toward its
  stations. The logsum is the accessibility measure the city-builder bridge
  needs, so this is the first upgrade when that work starts.

## 4. Desired arrival times

`work_arrival_profile` holds the share of commuters wanting to arrive at work
in each 15-minute bin from `arrival_profile_start_h [5:00]`. The default is a
bell from 5:00 to 10:00 peaking around 7:15 to 8:15, shaped like the ACS
"time arriving at work" table.

It drives two things:
1. **Peak-hour share.** `work_peak_hour_share()` is the busiest 4-bin window's
   share (about 0.35 by default). Peak-hour car demand = daily work car trips
   x that share + other car trips x `other_peak_hour_share [0.07]`. That is the
   demand loaded onto roads for congestion.
2. **Schedule delay on transit.** A commuter with a fixed arrival time on a line
   with headway `h` arrives early by `h/2` on average. That early time is added
   to transit work trips at `schedule_delay_weight [0.5]` (studies find early
   arrival costs about half of in-vehicle time). This is why a 30-minute
   commuter rail line loses more work riders than its wait time alone suggests.

Why not full departure-time choice: peak spreading (drivers shifting from 8:00
to 7:15) needs time-sliced assignment, which multiplies run time by the number
of slices. It belongs with the regional tier (mesoscopic, every 5 to 15 game
minutes) in M3. The profile is already in place to feed it.

## 5. Networks and skims

**Road graph.** Directed edges from `road_links` (two per link unless `oneway`).
Free-flow time = length / speed. Capacity per direction from the input, else
lanes x per-lane capacity by speed: 1,900 veh/h at 80+ km/h (freeway), 900 at
55+ (arterial), 600 below (local). Car skims (time, distance, tolls between
every pair of zones) come from one Dijkstra tree per origin zone, stopped as
soon as every zone's road node is reached. Same-zone or same-node trips use
the intrazonal distance x `detour_factor [1.3]` at 25 km/h.

**Transit graph.** One platform per (line, stop). Edges:
- Ride: between consecutive stops in both directions. Time = straight-line
  distance x mode alignment factor / running speed + dwell.
- Transfer: between platforms of different lines within `transfer_walk_km [0.4]`.
  Cost = walk + wait for the new line + `transfer_penalty_min [5]` + any fare
  difference.
- Walk access/egress: zone to platforms whose stop covers part of the zone.
- Park-and-ride access: drive from the origin to stops flagged `park_ride`
  within `pnr_max_drive_min [25]`, plus `pnr_park_min [3]`. Drive time comes from
  the car skim to the zone nearest the stop.

Expected wait `= h/2` for headways up to 12 minutes and `6 + (h - 12)/4` beyond:
riders time their arrival for infrequent service, so a 60-minute headway costs
an 18-minute wait, not 30.

Fares: a boarding pays the line's flat fare minus the highest fare already paid
on the trip (free transfers within a fare level, pay the difference to upgrade
from bus to rail or HSR), plus `fare_per_km` x distance ridden for distance-based
modes.

Paths are ranked by generalized minutes at `path_vot_per_hour [$18]`. The path's
components (in-vehicle, wait, walk, fare, transfers, last headway, drive time
and cost) are stored, and mode choice re-prices them with each zone's own value
of time.

**Stop coverage.** Walk-access transit is only available to the part of a zone
within `stop_walk_km [0.8]` of a stop. Coverage for one stop at distance `d`
from a zone of radius `r` is a piecewise-linear approximation of the circle
overlap area; a zone's coverage is the best single stop. Trips get walk transit
for the share `coverage_origin x coverage_destination`.

Why: centroid-to-stop distance alone would give a whole 3 km wide suburban block
group walk access from one stop on its edge, or none. Coverage keeps ridership
from jumping when a stop moves 100 m. Taking the best stop instead of a union
undercounts zones crossed by a long line; it is conservative and cheap.

## 6. Mode choice

Multinomial logit over six choices: drive alone, carpool, transit with walk
access, transit with drive access (park-and-ride), walk, bike.

`U_m = asc[m] + beta_per_min [-0.035] x GC_m`, `P_m = exp(U_m) / sum exp(U_k)`
over available choices.

Generalized cost in minutes (`vot` = the origin zone's $/minute):

| Choice | GC |
|---|---|
| Drive | car time + `car_terminal_min [3]` + (km x `car_cost_per_km [$0.125]` + tolls + parking x share) / vot |
| Carpool | same + `carpool_extra_min [5]`, money divided by `carpool_occupancy [2.3]` |
| Transit (walk) | in-vehicle + `wait_weight [2]` x wait + `walk_weight [2]` x walk + `transfer_penalty_min` x transfers + fare / vot + schedule delay (work trips) |
| Transit (drive) | drive minutes + the transit terms + (fare + drive cost) / vot |
| Walk | distance x detour / `walk_kph [4.8]`; only up to `max_walk_km [4]` |
| Bike | distance x detour / `bike_kph [15]`; only up to `max_bike_km [15]` |

Parking: the destination's daily cost x `parking_share_work [0.5]` per work trip
(the daily cost spread over the two trips) or `parking_share_other [0.25]`.

Car cost is perceived marginal cost (fuel and wear, about $0.20 per mile), not
ownership, because that is what people weigh trip by trip (the overview doc's
rule).

Wait and walk weigh double in-vehicle time, which matches observed behavior and
is why frequency matters more than top speed.

**Segments.** Each zone pair's trips are split four ways and the logit runs on
each: {car available, zero-car household} x {near a stop at both ends, not}.
Zero-car households cannot drive alone or park-and-ride; they can carpool (as a
passenger). Walk transit needs the near segment. Trips with no available choice
are counted as `unserved_trips`.

Work trips use congested (peak) car times; other trips use free-flow times.

**Constants.** `asc` defaults give a car-dominated split close to Texas norms.
`calibrate_mode_shares` adjusts them with the standard iterative update
`asc_m += ln(target_m / model_m) - ln(target_drive / model_drive)` (drive is the
reference) until the model reproduces target shares. On real data the targets
are ACS commute shares by county, and per-metro constants (the doc's plan).

Why one logit, not nested: a nested logit (transit nest) gives more realistic
substitution between transit sub-modes, but needs nesting parameters we cannot
calibrate yet. The MNL's equal-substitution property mainly overstates how much
a new transit line pulls from walk and bike, a small error in Texas.

## 7. Car assignment and feedback

Peak-hour vehicle demand (section 4) is loaded all-or-nothing onto shortest
paths. Link times follow the BPR curve
`t = t0 x (1 + bpr_alpha [0.15] x (v/c)^bpr_beta [4])`.

`equilibrate` starts from free-flow times and runs `feedback_iterations [4]`
rounds of:
1. car skims on current link times,
2. transit skims (park-and-ride drive times depend on car skims),
3. mode choice,
4. all-or-nothing assignment on the same shortest-path trees, averaged into
   link volumes with step `1/n` (method of successive averages),
5. new link times.

Then it skims once more, runs mode choice and loads transit.

`equilibrate_warm` (what the game runs once per game day) starts from the
previous day's volumes and runs `daily_iterations [1]` round with step
`1/(feedback_iterations + n)`, i.e. it treats the previous solution as four
rounds of history. The step is fixed at 1/5, so each game day moves road
volumes a fifth of the way toward the day's all-or-nothing target: a line edit's
effect on car times is about half absorbed after 3 game days and 90% after
10. That lag is deliberate. Demand does not change between days unless the
player changed lines, car volumes move a few percent per line, and drivers in
reality take days to weeks to re-sort their routes. A caller that needs the
settled state at once (tests, a scenario load) runs `equilibrate`.

Why MSA with feedback and not Frank-Wolfe user equilibrium: MSA with feedback
converges to the same combined mode-and-route equilibrium, needs no line
search, and folds mode choice into the same loop. Four rounds get volumes to
within a few percent; more rounds only cost time. Why one peak hour: congestion
is what drives mode choice for commuters, and the peak hour is where it lives.
Midday and evening congestion come with the regional tier's time slices.

## 8. Transit loading

Daily transit trips per zone pair:
`walk access: work(i->j) + work(j->i) + other(i->j)` (the PM return of a
commuter living in `j` and working in `i` travels `i->j`);
`drive access: 2 x work(i->j) + other(i->j)`.

Each origin's trips follow its best path. Boardings are counted at the first
stop and at each transfer (`boardings_by_stop`), segment loads per direction,
and passenger-km.

Simplifications and why:
- **All-or-nothing transit assignment** (one best path per pair, no frequency
  split across parallel lines). Simple and stable; matters only when two lines
  compete on the same corridor. Frequency-based (optimal strategy) assignment is
  the upgrade.
- **Park-and-ride returns** are loaded on the morning path, so they board where
  they boarded in the morning. Daily boardings are right; the direction split on
  segments is approximate.
- **No crowding feedback.** Loads above capacity are reported
  (`peak_hour_load` vs `peak_hour_capacity`) but do not yet deter riders. Next
  upgrade once lines can saturate.

## 9. Line economics

- Vehicles = ceil(round trip x 1.1 layover / headway).
- Revenue hours per day = vehicles x span (`span_hours`, default by mode).
- Operating cost = revenue hours x cost per car hour x cars per vehicle
  (`ModeDefaults::cost_per_revenue_hour()`). NTD counts rail revenue hours per
  car, so `cost_per_car_hour` is the published figure: bus $182, light rail
  $436 x 2 cars, commuter rail $716 x 2 cars (FTA NTD 2023 Texas agencies,
  [figures.csv](../../data-pipeline/reference/figures.csv)). Other modes are
  estimates from the design doc's ranges, one car per vehicle.
- Check: the synthetic Travis bench's first light rail line recovers about 4% of
  operating cost from fares, against 4.8 to 5.8% at DART and Houston METRO
  (`farebox_recovery_texas_big_city`).
- Fare revenue = boardings x fare + passenger-km x fare per km. This overstates
  revenue when riders transfer free; good enough until a fare ledger exists.
- Peak-hour load = busiest segment's daily load x the transit peak factor
  (work transit trips x peak-hour share, plus other trips x 0.07, over all
  transit trips). Capacity = 60 / headway x vehicle capacity.

Projected or foreign-only modes (maglev, gondola, people mover) take speeds and
costs from built systems abroad; these are flagged in the build menu by the app,
not here. No hyperloop.

## 10. Capital cost and funding

Collins decided (D30) that the network is funded by state and federal grants
plus a local tax base, using real sourced figures. Code:
[src/economy.rs](../src/economy.rs), wired into `Sim` in `model.rs`.

### Capital cost (F23, first cut)

`capital = capital_per_km x route length + vehicles x cars_per_vehicle x vehicle_cost`

where `vehicles` is the fleet from section 9 (round trip x 1.1 / headway).

| Mode | Per route km | Per car | Source |
|---|---|---|---|
| Bus | $0.3M (stops, shelters) | $716,400 | price: MTC FY2024-25 pricelist (figures.csv); per km estimate |
| Trolleybus | $3M (overhead wire) | $1,407,600 | price: MTC battery-electric bus as proxy; per km estimate |
| BRT | $15M | $1,407,600 | estimate; same bus price |
| Tram | $117M/mile = $72.7M | $5M | Eno: US mostly-surface rail; LRV price (figures.csv) |
| Light rail | **$840M/mile = $522M** | $5M | Austin Light Rail Phase 1, FTA profile (Collins's chosen figure); LRV price |
| Commuter rail | $20M | $5M | estimate (shares existing freight right-of-way) |
| Metro | $570M/mile = $354M | $15M | Eno: US mostly-tunnel rail; car price estimate |
| High-speed rail | $167M/mile = $104M | $40M per train | Texas Central (USDOT > $40B / 240 mi); train estimate |
| Monorail, people mover, gondola, maglev, ferry | $150M, $100M, $30M, $300M, $0 | $10M, $5M, $0.1M, $60M, $10M | estimates |

`tests/figures.rs` checks every sourced cell against figures.csv.

Simplifications and why:
- The Austin figure is a whole-project cost (stations, maintenance facility,
  vehicles, finance charges). Adding vehicles on top double-counts about 1-2%;
  not worth a second constant.
- No terrain, land cover or right-of-way multipliers yet (rest of F23). The
  same line costs the same through wetlands or downtown.
- Capital is charged the moment a line is added or extended: no construction
  period yet (F24). An edit pays only the cost above what the line already paid
  (`capital_paid` per line); shortening or removing a line refunds nothing,
  because sunk construction is gone.
- Lines in the input (present at start) are treated as already paid for.

### Federal capital grants

When capital is charged, the federal government pays `federal_capital_share
[0.5]` of it if the line passes a cost-effectiveness gate: total capital cost
per forecast daily boarding at most `federal_max_capital_per_daily_rider
[$600k]`. This stands in for FTA Capital Investment Grants, which fund up to 80%
(typically about half) of projects rated on cost-effectiveness. Austin Light
Rail Phase 1 ($7.1B excluding finance charges, 18,700 forecast riders) is about
$380k per rider and is in the CIG pipeline, so the gate sits above it. The
forecast is the model's own daily boardings with the line in place, the same
number `preview_line` shows.

### Monthly settlement

At each 30-day month boundary (`tick` or `Sim::skip_to_next_month`), the ledger
closes the month using that day's line results:

| Line | Formula |
|---|---|
| Fare revenue | 30 x sum of `fare_revenue_per_day` |
| Operating cost | 30 x sum of `operating_cost_per_day` |
| Operating assistance | operating cost x `operating_assistance_share [0.12]` (federal formula and state funds; estimate) |
| Local tax | served population x `taxable_sales_per_capita [$23,000]` x `local_sales_tax_rate [1%]` / 12 |

Served population is the residents of every zone with any walking coverage of
a stop. Texas transit authorities (CapMetro, DART, Houston METRO) levy a 1%
sales tax in their member cities; growing the network grows the taxing area.
Taxable sales per capita is an estimate (CapMetro's ~$300M/yr sales tax over
~1.4M residents implies ~$21k) pending a sourced figures.csv row.

Cash starts at `starting_cash [0]` and may go negative: bonds, debt service and
the bankruptcy rule are F26. On the synthetic Travis bench, a 28 km light rail
line costs about $16B and fails the grant gate at its uncalibrated ridership;
it then needs local tax of about $4M a month against $8.6M operating cost.

## Known gaps

In rough priority for M1 tuning:

1. Calibrate on real Travis data: ACS county mode shares, TxDOT counts, and a
   CapMetro Red Line replica within 30% of reported ridership (M1 exit test).
2. Destination choice by logsum (transit-aware trip distribution).
3. Transit crowding in path and mode choice.
4. Frequency-based transit assignment for shared corridors.
5. Time-sliced assignment and departure-time choice (regional tier, M3).
6. Statewide scale: Z x Z matrices are ~8 bytes per pair per matrix, fine for
   700 zones, not for 6,900 tracts (M3 needs sparse or blocked storage and a
   contracted highway graph).
7. Park-and-ride car trips are not added to road volumes.
8. Capital cost has no terrain or right-of-way multipliers and no construction
   period; funding has no bonds or state appropriation yet (F23, F24, F26).
