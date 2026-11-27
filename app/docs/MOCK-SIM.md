# Mock simulation

`MockSim` ([src/sim/mock.ts](../src/sim/mock.ts)) stands in for
`crates/sim-core` when the client runs in a plain browser (`npm run dev`).
The desktop app always uses sim-core (decision A12). The mock keeps UI work
possible without a Rust toolchain. It is
**deterministic, fast (a few ms per run), responsive to stop placement,
headway and fare, and not calibrated beyond one sanity point**. Do not tune
game balance against it. The ridership panel header says which backend
answered.

## Synthetic Travis County

`buildZones()` lays a ~2 km grid (0.02 degrees) over the Travis County
bounding box: 1,200 zones.

- Population: `6000 * exp(-d/12) + 200` per zone, `d` = km from downtown.
  Total about 1.30M (real county: ~1.3M).
- Jobs: a background `1500 * exp(-d/15)` plus eight job centers (Downtown,
  UT/Capitol, Domain, Airport, Mueller, Round Rock, Northwest Hills, South
  Lamar) each spread with a 1.5 km decay. Total about 710k.

These are shapes, not data. The real zones (block groups with LODES jobs)
come from the data pipeline in the sim-core `ScenarioInput` format.

## Model, per line

Two kinds only: **local** (bus, trolleybus, BRT) and **regional** (every
other mode).

| Parameter | Local | Regional | Why |
|---|---|---|---|
| In-vehicle speed | 18 km/h | 45 km/h | Typical urban bus vs commuter/light rail |
| Catchment radius `r` | 0.8 km | 2.5 km | Walk shed vs park-and-ride shed (most Texas suburban rail trips start with a drive) |
| Access + egress time | 4 min | 8 min | Short walk vs drive/park or long walk |
| Network factor | 4 | 1 | Stands in for riders transferring from the rest of a bus network the mock does not model |

1. **Catchments.** For each stop, people and jobs within reach are summed with
   a Gaussian weight `exp(-(d/r)^2)`. Each stop also gets `allJobs`: every job
   in the county weighted by `decay(d) = 1/max(d,1)^1.2`, the gravity
   denominator.
2. **Trip table.** For each ordered stop pair (i, j) more than 0.8 km apart
   along the line:
   `trips_ij = pop_i * 2.5 * jobs_j * decay(dist_ij) / allJobs_i`.
   That is: of the trips leaving i's catchment, the share whose destination
   is j's catchment.
3. **Mode split.** Binary logit, transit vs car, on generalized cost in
   minutes (value of time $18/h, the USDOT baseline used in the design doc):
   - transit = `2 * headway/2` (wait, double weight) + `2 * access`
     (double weight) + ride time + fare / VoT
   - car = 6 min (parking/access) + `1.3 * dist` at 40 km/h + `$0.25/km` / VoT
   - `share = 1 / (1 + exp(0.06 * (transit - car) + 2.0))`
4. **Results.** `trips_ij * share * networkFactor` is added to boardings at
   stop i. Daily riders = the sum. Every route is simulated independently: no
   competition between lines, no congestion, no transfers between drawn lines.

## Capital cost and budget

`mockCapital(route, riders)` mirrors sim-core's capital formula with its
sourced rows for bus ($0.3M/km, $716k per bus), light rail ($840M/mile,
$5M per car) and commuter rail ($20M/km, $5M per car); other modes use the bus
or commuter rail row. Fleet = round trip at the mock's speed divided by the
headway, plus one. The grant is 50% when capital per daily rider is at most
$600k, else 0. `MockSim` keeps a ledger with the same rules as sim-core:
charge capital less grant on build, charge only the increase on an edit,
refund nothing. There is no operating settlement in the mock, so
`statements` is always empty.

The mock ignores `offpeakHeadwayMin` and uses the peak headway for every trip.

## Calibration point

The Red Line demo at a 30-minute headway (the 10 real stations from the
pipeline's Travis build, MetroRail's $3.50 fare) returns about **1,970 weekday
riders**; the preset itself now uses 32 minutes (about 1,750 in the mock). `ASC_CAR = 2.0`
was tuned to land between APTA's ~1,500 a weekday and sim-core's 2,285 for
the same stations. A 10-stop bus line on the
Guadalupe/Lamar corridor gives about 1,100 at 15 minutes and 1,500 at
10 minutes, below the real 801/1-corridor numbers; acceptable for a mock.

Ridership is quite elastic to headway (Red Line: ~810 at 45 min,
~1,970 at 30, ~3,550 at 20, ~6,300 at 10). Real elasticities are lower; the design doc
already expects a tunable exaggeration factor for gameplay, which belongs in
sim-core, not here.

## Tests

`mock sim > returns plausible Red Line ridership that rises with frequency`
keeps the Red Line between 500 and 20,000 riders at 30-minute headway and
checks that 10-minute headway beats 30-minute.
