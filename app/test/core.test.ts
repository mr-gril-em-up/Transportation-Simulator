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
    expect(line.stops[0]).toMatchObject({ lon: RED_LINE[0].lngLat[0], lat: RED_LINE[0].lngLat[1], park_ride: true });
    const res = fromLineResult(
      {
        route_id: "red",
        daily_boardings: 2500.4,
        boardings_by_stop: r.stops.map((_, i) => i * 10.4),
        operating_cost_per_day: 50000,
        farebox_recovery: 0.08,
      },
      r,
    );
    expect(res.dailyRiders).toBe(2500);
    expect(res.boardingsByStop.s2).toBe(21);
    expect(res.fareboxRecovery).toBe(0.08);
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
