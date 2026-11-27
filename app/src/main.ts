import "maplibre-gl/dist/maplibre-gl.css";
import "./style.css";
import { GameClock } from "./clock";
import { RED_LINE, TEXAS_BOUNDS, TRAVIS_CENTER } from "./data/geo";
import { LineDrawer } from "./draw";
import { createMap, GEO_FULL, SCHEMATIC_FULL, setData, zoomBand } from "./map";
import { layoutSchematic, morph } from "./schematic";
import { isRegional, type LngLat, type Mode, type RouteResult, type RouteSpec } from "./sim/api";
import { createSim } from "./sim";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const COLORS = ["#dc241f", "#0098d4", "#00782a", "#9b0056", "#f3a9bb", "#ee7c0e", "#6950a1", "#b36305"];

const sim = createSim();
const map = await createMap("map");
if (import.meta.env.DEV) Object.assign(window, { __map: map }); // for e2e scripts
const routes: RouteSpec[] = [];
let results = new Map<string, RouteResult>();
let nextId = 1;

// ---- Simulation round trip -------------------------------------------------
async function apply(call: Promise<RouteResult[]>): Promise<void> {
  const t0 = performance.now();
  results = new Map((await call).map((r) => [r.routeId, r]));
  $("sim-ms").textContent = `sim ${Math.round(performance.now() - t0)} ms`;
  render();
  renderPanel();
}

function addRoute(mode: Mode, pts: LngLat[], stopNames?: string[], name?: string): void {
  const id = `r${nextId++}`;
  const n = routes.filter((r) => isRegional(r.mode) === isRegional(mode)).length + 1;
  const route: RouteSpec = {
    id,
    name: name ?? (isRegional(mode) ? `Rail ${n}` : `Bus ${n}`),
    mode,
    color: COLORS[(nextId - 2) % COLORS.length],
    stops: pts.map((p, i) => ({ id: `${id}-s${i}`, name: stopNames?.[i] ?? `Stop ${i + 1}`, lngLat: p })),
    headwayMin: isRegional(mode) ? 30 : 15,
    fare: 1.25,
  };
  routes.push(route);
  void apply(sim.upsertRoute(route));
}

// ---- Rendering --------------------------------------------------------------
function render(): void {
  const z = map.getZoom();
  const t = Math.min(1, Math.max(0, (z - SCHEMATIC_FULL) / (GEO_FULL - SCHEMATIC_FULL)));
  const pxPerDeg = (512 * 2 ** z) / 360;
  const rail = routes.filter((r) => isRegional(r.mode));
  const layouts = layoutSchematic(rail, pxPerDeg);
  const stops: GeoJSON.Feature[] = [];

  setData(
    map,
    "rail-lines",
    layouts.map((l) => ({
      type: "Feature",
      properties: { color: l.color },
      geometry: { type: "LineString", coordinates: l.schem.map((s, i) => morph(s, l.geo[i], t)) },
    })),
  );
  for (const l of layouts)
    for (const s of l.stops) {
      const b = results.get(l.routeId)?.boardingsByStop[s.id] ?? 0;
      stops.push(stopFeature(morph(s.schem, s.geo, t), true, l.color, s.name, b, s.interchange));
    }

  const bus = routes.filter((r) => !isRegional(r.mode));
  setData(
    map,
    "bus-lines",
    bus.map((r) => ({
      type: "Feature",
      properties: { color: r.color },
      geometry: { type: "LineString", coordinates: r.stops.map((s) => s.lngLat) },
    })),
  );
  for (const r of bus)
    for (const s of r.stops) {
      const b = results.get(r.id)?.boardingsByStop[s.id] ?? 0;
      stops.push(stopFeature(s.lngLat, false, r.color, s.name, b, false));
    }
  setData(map, "stops", stops);

  $("zoom-badge").innerHTML =
    `<b>${zoomBand(z)}</b> z${z.toFixed(1)} · ${t === 0 ? "schematic" : t === 1 ? "geographic" : "morphing"}` +
    ` <button data-go="tx">Texas</button><button data-go="travis">Travis</button>`;
}

function stopFeature(c: LngLat, regional: boolean, color: string, name: string, boardings: number, interchange: boolean): GeoJSON.Feature {
  return {
    type: "Feature",
    properties: { regional, color, boardings, interchange, label: boardings ? `${name} · ${boardings.toLocaleString()}` : name },
    geometry: { type: "Point", coordinates: c },
  };
}

function renderPanel(): void {
  const list = $("route-list");
  if (!routes.length) {
    list.innerHTML = "<em>No lines yet. Draw one.</em>";
    return;
  }
  list.innerHTML = routes
    .map((r) => {
      const riders = results.get(r.id)?.dailyRiders ?? 0;
      const opts = [5, 10, 15, 20, 30, 60]
        .map((h) => `<option value="${h}" ${h === r.headwayMin ? "selected" : ""}>every ${h} min</option>`)
        .join("");
      return `<div class="route" data-id="${r.id}">
        <span class="swatch" style="background:${r.color}"></span>
        <b>${r.name}</b> <span class="riders">${riders.toLocaleString()}</span> riders/day
        <div class="row"><select data-act="headway">${opts}</select>
        <button data-act="del" title="Delete line">✕</button></div></div>`;
    })
    .join("");
}

// Zoom re-renders only the map: the schematic morphs, the clock is untouched.
map.on("zoom", render);

// ---- UI wiring --------------------------------------------------------------
const drawer = new LineDrawer(
  map,
  (mode, pts) => addRoute(mode, pts),
  (drawing) => ($("draw-hint").hidden = !drawing),
);
$("btn-draw-bus").onclick = () => drawer.start("bus");
$("btn-draw-rail").onclick = () => drawer.start("light_rail");
$("btn-demo").onclick = () =>
  addRoute("commuter_rail", RED_LINE.map((s) => s.lngLat), RED_LINE.map((s) => s.name), "Red Line (demo)");

$("route-list").addEventListener("change", (e) => {
  const el = e.target as HTMLSelectElement;
  const r = routes.find((x) => x.id === el.closest<HTMLElement>(".route")?.dataset.id);
  if (r && el.dataset.act === "headway") {
    r.headwayMin = Number(el.value);
    void apply(sim.upsertRoute(r));
  }
});
$("route-list").addEventListener("click", (e) => {
  const el = e.target as HTMLElement;
  const id = el.closest<HTMLElement>(".route")?.dataset.id;
  if (id && el.dataset.act === "del") {
    routes.splice(routes.findIndex((x) => x.id === id), 1);
    void apply(sim.removeRoute(id));
  }
});
$("zoom-badge").addEventListener("click", (e) => {
  const go = (e.target as HTMLElement).dataset.go;
  if (go === "tx") map.fitBounds(TEXAS_BOUNDS, { padding: 40 });
  if (go === "travis") map.flyTo({ center: TRAVIS_CENTER, zoom: 11.5 });
});

// ---- Clock -------------------------------------------------------------------
const clock = new GameClock();
const pauseBtn = $("btn-pause");
const lapseBtn = $("btn-lapse");
const togglePause = () => {
  clock.paused = !clock.paused;
  pauseBtn.textContent = clock.paused ? "▶" : "⏸";
};
const cycleLapse = () => (lapseBtn.textContent = `Time-lapse ${clock.cycleLapse()}x`);
pauseBtn.onclick = togglePause;
lapseBtn.onclick = cycleLapse;
window.addEventListener("keydown", (e) => {
  if (drawer.active || e.target instanceof HTMLSelectElement) return;
  if (e.key === " ") togglePause();
  if (e.key === "t" || e.key === "T") cycleLapse();
});
let last = performance.now();
function frame(now: number) {
  clock.tick((now - last) / 1000);
  last = now;
  $("clock-time").textContent = clock.label();
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

render();
