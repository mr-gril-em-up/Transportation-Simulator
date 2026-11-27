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
- Ride: between consecutive stops in both directions. Time = segment km /
  running speed + dwell. Segment km is the street route for road modes (see
  below), else straight-line distance x the mode's `alignment_factor`.
- Transfer: between platforms of different lines within `transfer_walk_km [0.4]`.
  Cost = walk + wait for the new line + `transfer_penalty_min [5]` + any fare
  difference.
- Walk access/egress: zone to platforms whose stop covers part of the zone.
- Park-and-ride access: drive from the origin to stops flagged `park_ride`
  within `pnr_max_drive_min [25]`, plus `pnr_park_min [3]`. Drive time comes from
  the car skim to the zone nearest the stop.

**Road routing for bus, trolleybus and BRT (SIM-8).** Code:
[src/roads.rs](../src/roads.rs). Each stop snaps to the nearest routable road
node (a node of the contracted graph) within `SNAP_KM [0.5]`. Between two
snapped nodes, A* on free-flow distance (straight-line heuristic) finds the
shortest directed path; its km, plus the two stop-to-node hops, replaces
straight km x `alignment_factor`. The running speed stays the mode speed, so
congestion does not slow buses yet. The segment falls back to the factor if a
stop is over 0.5 km from a routable node, no path exists, or the route is over
`MAX_DETOUR [2.5]` x the straight line (a bad snap onto a one-way loop or across
a river). Routes are cached per node pair, so re-evaluating every line on an
edit only routes new stop pairs (a 50-stop bus adds no measurable time on real
Travis). Contracted edges are expanded through their member links back to node
coordinates, giving `LineResult::segment_shapes` for the app to draw.

Check on real Travis: routing 40 CapMetro bus routes from `transit.json` at
their OSM stops snaps 95% of 1,498 segments, with overall circuity (route km /
straight km) 1.27, median 1.22, 90th percentile 1.76. That is close to the bus
`alignment_factor [1.25]` it replaces, so the factor stays as the fallback.
Rail and other guided modes keep the factor: their alignments are not on the
road graph.

Expected wait `= h/2` for headways up to 12 minutes and `6 + (h - 12)/4` beyond:
riders time their arrival for infrequent service, so a 60-minute headway costs
an 18-minute wait, not 30.

Peak and off-peak headways (SIM-3): `headway_min` is the peak headway and
`offpeak_headway_min` (default the same) the midday and evening one. Work trips
are skimmed, chosen and loaded on peak headways; non-work trips on off-peak
headways. When no line has a different off-peak headway the two searches are
the same and run once. Simplification: each purpose sees one headway all day
(commuters the peak, everyone else the off-peak), not a time-of-day profile.

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

**Constants (F14, SIM-10).** `calibrate_mode_shares` fits them with the
standard iterative update
`asc_m += ln(target_m / model_m) - ln(target_drive / model_drive)` (drive is the
reference) until **commute** trips reproduce the targets; non-work trips use
the same constants. Transit is fitted as one share (walk- and drive-access
constants move together) unless the target splits it by access mode.

The defaults `asc [0, -2.44, -0.42, -4.18, -0.41, -3.89]` are fitted on real
Travis by `examples/calibrate.rs`:

- Data: release `data-48453-2023` with ACS block-group income and zero-car
  share (DATA-4), and today's CapMetro network from `transit.json` (74 route
  refs; the Red Line at its real 32/51-minute headways, buses at a placeholder
  20/30 minutes until GTFS headways land).
- Targets: ACS 2019-2023 B08301 Travis commute shares (figures.csv
  `acs_travis_commute_*`: drove alone 59.7%, carpool 7.5%, transit 1.67%, walk
  2.07%, bike 0.60%, work from home 27.2%, other 1.3%). Work from home and
  taxi/motorcycle/other are not modeled trips, so the five travel modes are
  renormalized by their 71.5% total: drive 83.5%, carpool 10.4%, transit 2.3%,
  walk 2.9%, bike 0.8%. The fit reproduces them to 0.1 point (F14 target: 2).
- Park-and-ride share of transit commuters: 3%. **Tuned, not sourced**: ACS
  has no access mode and no CapMetro on-board survey split was found. It
  decides the Red Line: with walk and drive access fitted together, modeled
  park-and-ride put 6k-16k riders a day on it against ~1.5k real; at 8% of
  transit it carries 4,265, at 3% 1,917, at 1% 1,024. 3% is the value that puts
  the Red Line inside the F19 band, so **F19 is no longer an independent
  check**.
- Independent check: modeled bus boardings, 65.5k a day, against CapMetro's
  ~69-70k weekday bus boardings (NTD 2023: 75,447 weekday all modes, bus ~92%
  of annual trips), 6% low, with placeholder bus headways.

Caveat: the constants absorb everything the model misses (no Williamson County
zones at the Red Line's north end, placeholder bus frequencies, no transit
crowding), so refit after any of those change. Re-run:
`cargo run --release -p sim-core --example calibrate -- scenario.json transit.json`.

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
- Revenue hours per day = peak vehicles x `peak_service_hours [6]` + off-peak
  vehicles x (span - 6), where vehicles = ceil(round trip x 1.1 / headway) for
  each headway and span is `span_hours` (default by mode). The fleet
  (`vehicles_required`, which capital pays for) is the larger of the two, so a
  shorter off-peak headway is not free.
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
- Capital is charged when construction starts, all at once rather than spread
  over the build. An edit pays only the cost above what the line already paid
  (`capital_paid` per line); shortening or removing a line refunds nothing,
  because sunk construction is gone.

### Construction period (F24)

Decision SIM-1. A new line, or a new alignment of an existing one, opens on
`clock.day() + ceil(build_years x 365 / construction_speedup [12])`.

| Mode | Real years | Game days at 12x |
|---|---|---|
| Bus | 0.5 | 16 |
| Trolleybus, ferry | 1.5, 1 | 46, 31 |
| BRT, gondola | 2.5, 2 | 77, 61 |
| Tram, commuter rail, people mover | 4 | 122 |
| Monorail | 5 | 153 |
| Light rail | 6 | 183 |
| Metro | 9 | 274 |
| High-speed rail, maglev | 10 | 305 |

Light rail's 6 years is the roadmap's F24 check and matches Austin Phase 1
(FTA engineering 2027, revenue service 2033); the rest are estimates.

- Until it opens, a line carries no riders and is listed in `pending_lines`.
  The federal grant is decided at the start on the forecast ridership (what
  `preview_line` shows), as FTA rates projects before construction.
- `update_line` treats a change of mode or stop locations as construction (the
  old version keeps running until the new one opens). Headway, fare, speed,
  span and name changes apply at once, to the running version and to any
  version under construction. Every edit charges capital above what the line
  already paid, so a headway cut pays for its extra vehicles.
- `params.instant_build` skips construction. It defaults to **false** (SIM-4):
  the app shows lines under construction from `pending_lines` and the
  `preview_line` forecast (APP-3). Tests, the bench and the Red Line probe set
  it true.
- Simplifications: build time does not grow with length, and roads under
  works keep full capacity. Both are F24 follow-ups.
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
| Operating assistance | operating cost x `operating_assistance_share [0.143]` (federal formula funds; Texas gives big-city transit authorities no state operating money) |
| Local tax | served population x `taxable_sales_per_capita [$20,600]` x `local_sales_tax_rate [1%]` / 12 |

Served population (the tax area, SIM-9) is the full population of every zone
whose centre lies within `tax_area_km [3]` of an open stop. Texas transit
authorities (CapMetro, DART, Houston METRO) levy a 1% sales tax across whole
member cities, not just the blocks beside a stop; the buffer stands in for
city membership, which follows service. Growing the network grows the taxing
area, up to the whole county (CapMetro's real base: ~1.36M residents, ~$280M a
year).

Why 3 km and not walking coverage: the first cut (#20) taxed only residents in
walking range, weighted by stop coverage. On real Travis that gave the Red
Line's ten stations ~$0.5M a month against $1.6M operating cost, so the first
rail line always bankrupted the player. With a 3 km buffer the same stations
tax ~291k residents, about $5.0M a month ($60M a year). That runs the line and
services a bond of a few hundred million, while a $1B commuter rail build from
scratch stays out of reach without a federal grant or a wider network first.
Buffer sizes on real Travis around the Red Line: 1 km 38k residents, 2 km
130k, 3 km 291k, 5 km 513k, 8 km 731k. 3 km is a balance choice.
Sources (figures.csv rows of the same names): the 1% rate is the Comptroller's
MTA table (the statutory cap). Taxable sales per capita is derived from
CapMetro's NTD 2023 local funds divided by 1% and by its 1.36M service-area
residents (Austin MSA high: $26,000). Operating assistance is CapMetro NTD
2023, federal $50.9M over $356.3M; 2023 may still carry pandemic relief, so it
is an upper bound, and small agencies run far higher (El Paso 57%).

Cash starts at `starting_cash [0]` and may go negative, but a month that ends
below zero counts toward bankruptcy (next section). On the synthetic Travis bench, a 28 km light rail
line costs about $16B and fails the grant gate at its uncalibrated ridership;
its tax area (before SIM-9, the 67k residents in walking range) then collected about $1.2M a month in local tax against $8.6M operating cost.

### Bonds, debt service and bankruptcy (F26)

`Sim::issue_bond(principal)` borrows now and repays in equal monthly
payments (a level-payment annuity, like a mortgage):

`payment = P x r / (1 - (1 + r)^-n)`, with `r = bond_rate [5.43%] / 12` and
`n = bond_term_months()` = `bond_term_years [30] x 12 / bond_term_speedup [1]` = 360.

The cash arrives in the month of issue (`MonthStatement::bond_proceeds`).
Payments start at the next month's settlement; each splits into interest on
the remaining balance and principal (`debt_service`, `interest`). The last
payment clears the balance and the bond leaves `Ledger::bonds`.
`Ledger::debt()` is the principal still owed.

**Borrowing cap (additional-bonds test, SIM-7).** Revenue bonds are secured
by the transit sales tax, and a new issue must leave annual debt service, old
plus new, within the pledged revenue divided by a coverage ratio:

`12 x (monthly_debt_service + new payment) <= tax_capacity() / bond_coverage_min [2.0]`

`tax_capacity()` is the annual tax of the tax area of every stop on lines
open **or under construction** (SIM-9): what the authority will collect once
its committed network runs, so a line can be bonded while it is still being
built. With no lines it is 0 and nothing can be borrowed. (SIM-7 first used
the whole scenario's population; that let the player borrow against tax the
network would never collect and walk into bankruptcy.) `Sim::bond_capacity()`
is the principal still allowed; `issue_bond` refuses above it. Real Travis:
the Red Line alone pledges ~$60M a year, so about $440M of 30-year debt at
5.43%; a county-wide network pledges $268M, about $2.0B.
Source (figures.csv `bond_coverage_min`): Houston METRO's sales-tax bonds
require 2.0x maximum annual debt service before new parity bonds (KBRA AAA
affirmation, Sep 2023); DART's senior lien is reported at 2x too.

Bankruptcy is the sandbox's only fail state (D15). At each settlement a month
is **distressed** if either:

- debt service exceeds the operating surplus (fares + operating assistance +
  local tax - operating cost, `MonthStatement::operating_surplus`), the
  roadmap's rule, a debt-service coverage ratio below 1. A bond pays nothing
  during its construction period (below), so only earning lines carry debt
  service; or
- cash ends the month below zero (SIM-5), tested every month.

A healthy month resets `distressed_months`. `bankruptcy_months [12]`
distressed months in a row set `Ledger::bankrupt_month`; the game is over.
sim-core keeps simulating so the app can show what went wrong, but
`issue_bond` refuses from then on. Ending the game is the app's call.

**Capitalized interest (SIM-6, SIM-7, SIM-9).** A line under construction has
no riders and is not yet in the tax area, so paying a bond during the build
would fail the coverage test, and drain cash, every month until opening. Real
projects capitalize interest: the issue covers its own interest until the
asset earns. Each bond records `coverage_from_month`, the month after the last
line under construction at issue opens. Before it, each month's interest is
added to the balance (`MonthStatement::capitalized_interest`, no cash moves)
and the level payment is recomputed over the payments still to come; from it
on, the bond pays normally and its payments count in the coverage test. The
grace is fixed at issue, so later construction does not extend it (bounded by
one build period). History: SIM-6 skipped the coverage test while anything
was under construction (stretchable forever); SIM-7 fixed the grace per bond
but still drew payments from cash during construction, which pushed a
fully-bonded build below zero; SIM-9 capitalizes the interest instead.

Why cash counts: with only the debt-service test, a player could skip bonds
and run cash negative forever, an interest-free loan. Counting negative cash
makes bonds the way to pay for capital and makes running deficits fatal.

Why real 30-year terms (SIM-5): D9 said bond terms compress with
construction. Only construction is compressed; fares, costs and taxes are per
real day. Compressing the term 12x alone makes each payment ~12x larger, so a
tax base that carries $X of 30-year debt carries about X/7 at 2.5 years.
Rough Travis scale: all 1.3M residents in the tax area would raise
1.3M x $20,600 x 1% = $268M a year; at 5.43% over 30 years that services at most
about $4.0B (annuity factor 14.8), versus about $0.6B over 2.5 years. Austin's
light rail is priced in billions, so compression would rule it out.
`bond_term_speedup` is the switch if Collins prefers D9 as written; the
question is with him.

Sources (figures.csv rows of the same names): `bond_rate` is the Bond Buyer
Revenue Bond Index (25 revenue bonds maturing in 30 years) for 29 Oct 2026,
5.43%, with the 2026 range 4.92-5.47% as low/high. It fits Austin Transit
Partnership's contract revenue bonds, which are not yet rated; a 30-year AAA
general obligation yielded 4.81%. `bond_term_years` 30 is the market
convention (unverified): ATP's January 2026 resolution caps maturities at 40
years and ATP had no bonds outstanding at the end of FY2025.

Balance note: the borrowing cap and the tax both grow with the network's
reach, so cheap, wide coverage (bus networks) builds the tax base and the
capacity to bond the expensive rail line that follows. Capital is charged
when a line is drawn whether or not it can be financed; a build the cash and
`bond_capacity()` cannot cover leaves cash negative and leads to bankruptcy,
so the app should show both before the player commits.

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
8. Capital cost has no terrain or right-of-way multipliers; construction time
   ignores length and roads under works keep full capacity; bankruptcy only
   blocks new bonds (its consequence is a pending design call); bonds cannot
   be refinanced (F23, F24, F26). There is no state appropriation because Texas gives big-city transit
   authorities none (figures.csv `operating_assistance_share` note).
