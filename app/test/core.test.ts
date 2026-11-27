import { describe, expect, it } from "vitest";
import { GameClock } from "../src/clock";
import { RED_LINE } from "../src/data/geo";
import { elbow, layoutSchematic } from "../src/schematic";
import type { RouteSpec } from "../src/sim/api";
import { MockSim } from "../src/sim/mock";

const redLine = (headwayMin = 20): RouteSpec => ({
  id: "red",
  name: "Red",
  mode: "commuter_rail",
  color: "#f00",
  headwayMin,
  fare: 1.25,
  stops: RED_LINE.map((s, i) => ({ id: `s${i}`, name: s.name, lngLat: s.lngLat })),
});

describe("clock", () => {
  it("runs 1 game minute per real second at 1x and scales with time-lapse only", () => {
    const c = new GameClock();
    const start = c.minutes;
    c.tick(1);
    expect(c.minutes - start).toBe(1);
    c.cycleLapse(); // 2x
    c.cycleLapse(); // 10x
    c.tick(1);
    expect(c.minutes - start).toBe(11);
    c.paused = true;
    c.tick(5);
    expect(c.minutes - start).toBe(11);
  });
});

describe("schematic", () => {
  const octilinear = (a: number[], b: number[]) => {
    const dx = Math.abs(b[0] - a[0]);
    const dy = Math.abs(b[1] - a[1]);
    return dx < 1e-9 || dy < 1e-9 || Math.abs(dx - dy) < 1e-9;
  };

  it("elbow connects any two points with 45/90 degree legs", () => {
    const a: [number, number] = [0, 0];
    const b: [number, number] = [3, 1];
    const pts = [a, ...elbow(a, b), b];
    for (let i = 1; i < pts.length; i++) expect(octilinear(pts[i - 1], pts[i])).toBe(true);
  });

  it("lays out the Red Line with one point per stop and keeps it near its real location", () => {
    const [l] = layoutSchematic([redLine()], 300);
    expect(l.stops).toHaveLength(RED_LINE.length);
    expect(l.geo).toHaveLength(l.schem.length);
    const cx = l.stops.reduce((a, s) => a + s.schem[0], 0) / l.stops.length;
    expect(cx).toBeGreaterThan(-98);
    expect(cx).toBeLessThan(-97.5);
  });

  it("merges shared stops into one interchange", () => {
    const a = redLine();
    const b: RouteSpec = {
      ...a,
      id: "blue",
      stops: [
        { id: "b0", name: "East", lngLat: [-97.6, 30.3] },
        { id: "b1", name: "Highland", lngLat: RED_LINE[3].lngLat },
        { id: "b2", name: "West", lngLat: [-97.85, 30.33] },
      ],
    };
    const out = layoutSchematic([a, b], 300);
    const hl = out.flatMap((l) => l.stops).filter((s) => s.name === "Highland");
    expect(hl.every((s) => s.interchange)).toBe(true);
    expect(hl[0].schem).toEqual(hl[1].schem);
  });
});

describe("mock sim", () => {
  it("returns plausible Red Line ridership that rises with frequency", async () => {
    const sim = new MockSim();
    const slow = (await sim.upsertRoute(redLine(30)))[0].dailyRiders;
    const fast = (await sim.upsertRoute(redLine(10)))[0].dailyRiders;
    expect(slow).toBeGreaterThan(500);
    expect(slow).toBeLessThan(20000);
    expect(fast).toBeGreaterThan(slow);
  });
});

describe("tauri adapter", () => {
  it("maps RouteSpec to sim-core TransitLine and per-stop results back by stop id", async () => {
    const { toTransitLine, fromLineResult } = await import("../src/sim/tauri");
    const r = redLine();
    const line = toTransitLine(r);
    expect(line.mode).toBe("commuter_rail");
    expect(line.headway_min).toBe(20);
    expect("offpeak_headway_min" in line).toBe(false); // omitted: sim-core uses the peak headway
    expect(toTransitLine({ ...r, headwayMin: 32, offpeakHeadwayMin: 51 })).toMatchObject({ headway_min: 32, offpeak_headway_min: 51 });
    expect(line.stops[0]).toMatchObject({ lon: RED_LINE[0].lngLat[0], lat: RED_LINE[0].lngLat[1], park_ride: true });
    const res = fromLineResult(
      {
        route_id: "red",
        daily_boardings: 2500.4,
        boardings_by_stop: r.stops.map((_, i) => i * 10.4),
        operating_cost_per_day: 50000,
        farebox_recovery: 0.08,
        length_km: 50,
        run_min: 60,
        vehicles_required: 6,
        passenger_km: 40000,
        segment_loads: [[100, 90]],
        peak_hour_load: 150,
        peak_hour_capacity: 1200,
      },
      r,
    );
    expect(res.dailyRiders).toBe(2500);
    expect(res.boardingsByStop.s2).toBe(21);
    expect(res.fareboxRecovery).toBe(0.08);
    expect(res.detail).toMatchObject({ lengthKm: 50, vehicles: 6, segmentLoads: [[100, 90]], peakHourCapacity: 1200 });
  });
});

describe("sim call queue", () => {
  it("finishes calls in call order even when an earlier call is slower, and survives failures", async () => {
    const { serialQueue } = await import("../src/sim/queue");
    const q = serialQueue();
    const done: string[] = [];
    const wait = (ms: number, tag: string, fail = false) => () =>
      new Promise<string>((res, rej) => setTimeout(() => (done.push(tag), fail ? rej(new Error(tag)) : res(tag)), ms));
    const a = q(wait(30, "a"));
    const b = q(wait(1, "b", true)).catch(() => "b-failed");
    const c = q(wait(1, "c"));
    expect(await Promise.all([a, b, c])).toEqual(["a", "b-failed", "c"]);
    expect(done).toEqual(["a", "b", "c"]);
  });
});

describe("mock desire lines", () => {
  it("returns the requested number of pairs, largest first", async () => {
    const lines = await new MockSim().desireLines(20);
    expect(lines).toHaveLength(20);
    for (let i = 1; i < lines.length; i++) expect(lines[i - 1].trips).toBeGreaterThanOrEqual(lines[i].trips);
  });
});

describe("stop editing", () => {
  it("inserts a stop where it lengthens the line least", async () => {
    const { bestInsertIndex } = await import("../src/edit");
    const line: [number, number][] = [
      [-97.75, 30.25],
      [-97.75, 30.3],
      [-97.75, 30.35],
    ];
    expect(bestInsertIndex(line, [-97.751, 30.27])).toBe(1); // between 0 and 1
    expect(bestInsertIndex(line, [-97.749, 30.33])).toBe(2); // between 1 and 2
    expect(bestInsertIndex(line, [-97.75, 30.2])).toBe(0); // before the first stop
    expect(bestInsertIndex(line, [-97.75, 30.4])).toBe(3); // after the last stop
  });
});

describe("build cost (F16)", () => {
  it("previews without charging, charges on build, and charges only the extra on extension", async () => {
    const s = new MockSim();
    const short = { ...redLine(), stops: redLine().stops.slice(0, 4) };
    const p = await s.previewRoute(short);
    expect(p.capitalCost).toBeGreaterThan(0);
    expect((await s.budget()).cash).toBe(0);

    await s.upsertRoute(short);
    const b1 = await s.budget();
    expect(b1.capitalSpentThisMonth).toBeCloseTo(p.capitalCost);
    expect(b1.cash).toBeCloseTo(p.federalGrant - p.capitalCost);

    const full = redLine();
    const pf = await s.previewRoute(full);
    await s.upsertRoute(full);
    expect((await s.budget()).capitalSpentThisMonth).toBeCloseTo(pf.capitalCost);

    await s.upsertRoute(short); // shrinking refunds nothing
    expect((await s.budget()).capitalSpentThisMonth).toBeCloseTo(pf.capitalCost);
  });

  it("maps sim-core's LinePreview and Ledger", async () => {
    const { fromPreview, fromLedger } = await import("../src/sim/tauri");
    expect(
      fromPreview({ line: { daily_boardings: 1234.6, operating_cost_per_day: 900, length_km: 7 }, capital_cost: 5e6, federal_grant: 2.5e6, opens_day: 183 }),
    ).toEqual({ dailyRiders: 1235, capitalCost: 5e6, federalGrant: 2.5e6, operatingCostPerDay: 900, opensDay: 183, lengthKm: 7 });
    const shape: [number, number][][] = [[[0, 0], [0.5, 0.2], [1, 0]]];
    expect(
      fromPreview({ line: { daily_boardings: 1, operating_cost_per_day: 1, length_km: 2, segment_shapes: shape }, capital_cost: 1, federal_grant: 0, opens_day: 0 }).shape,
    ).toEqual(shape);
    expect(
      fromLedger({
        cash: -1,
        current: { capital_spent: 2, federal_capital_grants: 1 },
        history: [0, 1, 2].map((month) => ({
          month,
          fare_revenue: 10,
          operating_cost: 100,
          operating_assistance: 30,
          local_tax: 50,
          capital_spent: 0,
          federal_capital_grants: 0,
          bond_proceeds: 0,
          debt_service: 4,
          interest: 3,
          capitalized_interest: 7,
          net: -14,
        })),
        distressed_months: 3,
        bankrupt_month: null,
        debt: 5,
        debt_service_per_month: 0.5,
        bankruptcy_months: 12,
        bond_rate: 0.0543,
        bond_term_months: 360,
        bond_capacity: 2e8,
        tax_capacity: 6e7,
      }),
    ).toEqual({
      cash: -1,
      capitalSpentThisMonth: 2,
      grantsThisMonth: 1,
      debt: 5,
      debtServicePerMonth: 0.5,
      distressedMonths: 3,
      bankruptcyMonths: 12,
      bankruptMonth: null,
      bondRate: 0.0543,
      bondTermMonths: 360,
      bondCapacity: 2e8,
      taxCapacity: 6e7,
      statements: [2, 1, 0].map((month) => ({
        month,
        fares: 10,
        operatingCost: 100,
        operatingAssistance: 30,
        localTax: 50,
        capitalSpent: 0,
        federalGrants: 0,
        bondProceeds: 0,
        debtService: 4,
        interest: 3,
        capitalizedInterest: 7,
        net: -14,
      })),
    });
  });

  it("mock bonds add cash and debt with a level monthly payment", async () => {
    const s = new MockSim();
    const b = await s.issueBond(100e6);
    expect(b.cash).toBe(100e6);
    expect(b.debt).toBe(100e6);
    expect(b.debtServicePerMonth).toBeCloseTo(563_440, -2); // 30-year annuity at 5.43%
    await expect(s.issueBond(0)).rejects.toThrow();
  });

  it("formats money for the HUD", async () => {
    const { money } = await import("../src/format");
    expect(money(820_400)).toBe("$820k");
    expect(money(34_520_000)).toBe("$34.5M");
    expect(money(-1.2e9)).toBe("-$1.20B");
  });
});

describe("sim-owned clock (F21)", () => {
  it("adopts the sim's time, pause and speed", () => {
    const c = new GameClock();
    c.sync({ gameSeconds: 86400 + 8 * 3600, timelapse: 10, paused: true, day: 1 });
    expect(c.label()).toBe("Day 2 08:00");
    expect(c.speed).toBe(10);
    c.tick(5);
    expect(c.label()).toBe("Day 2 08:00"); // paused
  });

  it("mock clock pauses, changes speed and skips to the next 30-day month", async () => {
    const s = new MockSim();
    const a = await s.setClock({ paused: true, timelapse: 60 });
    expect(a).toMatchObject({ paused: true, timelapse: 60, day: 0 });
    expect((await s.skipToNextMonth()).day).toBe(30);
    expect((await s.skipToNextMonth()).day).toBe(60);
  });

  it("maps the shell's ClockOut", async () => {
    const { fromClock } = await import("../src/sim/tauri");
    expect(fromClock({ game_seconds: 18000, timelapse: 2, paused: false, day: 0 })).toEqual({
      gameSeconds: 18000,
      timelapse: 2,
      paused: false,
      day: 0,
    });
  });
});

describe("scenario source label", () => {
  it("names the real Travis build, the synthetic region, or another file", async () => {
    const { sourceLabel } = await import("../src/sim/tauri");
    expect(sourceLabel("synthetic")).toBe("sim-core · synthetic region");
    expect(sourceLabel("/x/crates/data-pipeline/src-tauri/../../crates/data-pipeline/data/out/48453-2023/scenario.json")).toBe(
      "sim-core · Travis data",
    );
    expect(sourceLabel("C:\\data\\harris\\scenario.json")).toBe("sim-core · harris/scenario.json");
  });
});

describe("line inspector", () => {
  const stops = [
    { name: "A", boardings: 100 },
    { name: "B <x>", boardings: 50 },
    { name: "C", boardings: 150 },
  ];
  const detail = {
    lengthKm: 12.34,
    runMin: 31.4,
    vehicles: 6,
    passengerKm: 9000,
    segmentLoads: [
      [120, 80],
      [60, 140],
    ] as [number, number][],
    peakHourLoad: 300,
    peakHourCapacity: 200,
  };

  it("shows length, time, fleet and peak use, warning over capacity", async () => {
    const { inspectorHtml, peakUse } = await import("../src/inspector");
    expect(peakUse(detail)).toBe(1.5);
    const html = inspectorHtml(stops, detail);
    expect(html).toContain("12.3 km · 31 min end to end · 6 vehicles");
    expect(html).toContain("300 riders for 200 places (150%)");
    expect(html).toContain("Over capacity");
    expect(inspectorHtml(stops, { ...detail, peakHourLoad: 100 })).not.toContain("Over capacity");
  });

  it("draws one bar per stop and a load step per segment, escaping names", async () => {
    const { inspectorHtml } = await import("../src/inspector");
    const html = inspectorHtml(stops, detail);
    expect(html.match(/<rect x="[^"]*" y="[^"]*" width="[^"]*" height="[^"]*"><title>/g)).toHaveLength(3);
    expect(html).toContain("B &#60;x&#62; to C: 60 out, 140 in");
    expect(html).not.toContain("<x>");
  });
});

describe("street paths (SIM-8)", () => {
  it("joins segment shapes, falls back to straight segments, ignores stale shapes", async () => {
    const { routePath } = await import("../src/geo");
    const stops: [number, number][] = [
      [0, 0],
      [1, 0],
      [2, 0],
    ];
    expect(routePath(stops)).toEqual(stops);
    expect(
      routePath(stops, [
        [
          [0, 0],
          [0.5, 0.3],
          [1, 0],
        ],
        [],
      ]),
    ).toEqual([
      [0, 0],
      [0.5, 0.3],
      [1, 0],
      [2, 0],
    ]);
    expect(routePath(stops, [[]])).toEqual(stops); // one shape for two segments: stale
    // Stop 1 dragged to [1, 1] with the same stop count: both segments touching it go straight.
    const moved: [number, number][] = [
      [0, 0],
      [1, 1],
      [2, 0],
    ];
    const old: [number, number][][] = [
      [
        [0, 0],
        [0.5, 0.3],
        [1, 0],
      ],
      [
        [1, 0],
        [1.5, -0.2],
        [2, 0],
      ],
    ];
    expect(routePath(moved, old)).toEqual(moved);
    expect(routePath(stops, old)).toEqual([
      [0, 0],
      [0.5, 0.3],
      [1, 0],
      [1.5, -0.2],
      [2, 0],
    ]);
  });
});

describe("lines present at start (F44)", () => {
  it("map to routes with backend ids, stop ids and the mode's default fare", async () => {
    const { fromExisting } = await import("../src/sim/tauri");
    const r = fromExisting({
      route_id: "e3",
      line: {
        name: "550",
        mode: "commuter_rail",
        stops: [
          { lon: -97.7, lat: 30.2, name: "Downtown", park_ride: true },
          { lon: -97.8, lat: 30.6, name: "", park_ride: true },
        ],
        headway_min: 32,
        offpeak_headway_min: 51,
        fare: null,
      },
    });
    expect(r).toMatchObject({ id: "e3", name: "Red Line", mode: "commuter_rail", headwayMin: 32, offpeakHeadwayMin: 51, fare: 3.5 });
    expect(fromExisting({ route_id: "e0", line: { name: "801", mode: "bus", stops: [], headway_min: 10, fare: null } }).name).toBe("Route 801");
    expect(r.stops.map((s) => [s.id, s.name])).toEqual([
      ["e3-s0", "Downtown"],
      ["e3-s1", "Stop 2"],
    ]);
  });
});
