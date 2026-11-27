import "maplibre-gl/dist/maplibre-gl.css";
import "./style.css";
import { GameClock } from "./clock";
import { RED_LINE, TEXAS_BOUNDS, TRAVIS_CENTER } from "./data/geo";
import { LineDrawer } from "./draw";
import { StopEditor } from "./edit";
import { createMap, GEO_FULL, SCHEMATIC_FULL, setData, zoomBand } from "./map";
import { layoutSchematic, morph } from "./schematic";
import { isRegional, type LngLat, type Mode, type RouteResult, type RouteSpec } from "./sim/api";
import { createSim } from "./sim";
import { serialQueue } from "./sim/queue";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const escapeHtml = (s: string) => s.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
const COLORS = ["#dc241f", "#0098d4", "#00782a", "#9b0056", "#f3a9bb", "#ee7c0e", "#6950a1", "#b36305"];

const sim = createSim();
const map = await createMap("map");
if (import.meta.env.DEV) Object.assign(window, { __map: map }); // for e2e scripts
const routes: RouteSpec[] = [];
let results = new Map<string, RouteResult>();
let nextId = 1;

// ---- Simulation round trip -------------------------------------------------
// Sim calls run one at a time, in order. Each response is the full state after
// that call, so if two calls overlapped, an earlier call's late response could
// overwrite a newer one (review follow-up on #9).
const queued = serialQueue();

function apply(call: () => Promise<RouteResult[]>): Promise<void> {
  return queued(() => applyNow(call)).then(refreshDesire);
}

async function applyNow(call: () => Promise<RouteResult[]>): Promise<void> {
  const t0 = performance.now();
  // The first call can wait seconds while sim-core builds the scenario at startup.
  $("sim-ms").textContent = `${sim.label} · running…`;
  try {
    results = new Map((await call()).map((r) => [r.routeId, r]));
    $("sim-ms").textContent = `${sim.label} · ${Math.round(performance.now() - t0)} ms`;
  } catch (e) {
    $("sim-ms").textContent = `${sim.label} error: ${e}`;
    console.error(e);
  }
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
  void apply(() => sim.upsertRoute(route));
}

// ---- Rendering --------------------------------------------------------------
function render(): void {
  const z = map.getZoom();
  // While editing, draw true geography so dragged stops land where the cursor is.
  const t = editor.active ? 1 : Math.min(1, Math.max(0, (z - SCHEMATIC_FULL) / (GEO_FULL - SCHEMATIC_FULL)));
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
    l.stops.forEach((s, idx) => {
      const b = results.get(l.routeId)?.boardingsByStop[s.id] ?? 0;
      stops.push(stopFeature(morph(s.schem, s.geo, t), true, l.color, s.name, b, s.interchange, l.routeId, idx));
    });

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
    r.stops.forEach((s, idx) => {
      const b = results.get(r.id)?.boardingsByStop[s.id] ?? 0;
      stops.push(stopFeature(s.lngLat, false, r.color, s.name, b, false, r.id, idx));
    });
  setData(map, "stops", stops);

  $("zoom-badge").innerHTML =
    `<b>${zoomBand(z)}</b> z${z.toFixed(1)} · ${t === 0 ? "schematic" : t === 1 ? "geographic" : "morphing"}` +
    ` <button data-go="tx">Texas</button><button data-go="travis">Travis</button>`;
}

function stopFeature(
  c: LngLat,
  regional: boolean,
  color: string,
  name: string,
  boardings: number,
  interchange: boolean,
  routeId: string,
  idx: number,
): GeoJSON.Feature {
  return {
    type: "Feature",
    // routeId + idx let the stop editor find which stop was grabbed.
    properties: { regional, color, boardings, interchange, routeId, idx, editing: routeId === editor.route?.id, label: boardings ? `${name} · ${boardings.toLocaleString()}` : name },
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
      const res = results.get(r.id);
      const riders = res?.dailyRiders ?? 0;
      const econ =
        res?.operatingCostPerDay !== undefined
          ? `<div class="econ">$${Math.round(res.operatingCostPerDay).toLocaleString()}/day to run · farebox ${Math.round((res.fareboxRecovery ?? 0) * 100)}%</div>`
          : "";
      const opts = [5, 10, 15, 20, 30, 60]
        .map((h) => `<option value="${h}" ${h === r.headwayMin ? "selected" : ""}>every ${h} min</option>`)
        .join("");
      const editing = editor.route?.id === r.id;
      return `<div class="route${editing ? " editing" : ""}" data-id="${r.id}">
        <span class="swatch" style="background:${r.color}"></span>
        <b class="name" data-act="rename" title="Double-click to rename">${escapeHtml(r.name)}</b> <span class="riders">${riders.toLocaleString()}</span> riders/day${econ}
        <div class="row"><select data-act="headway">${opts}</select>
        <button data-act="edit">${editing ? "Done" : "Edit stops"}</button>
        <button data-act="del" title="Delete line">✕</button></div></div>`;
    })
    .join("");
}

// Startup: sim-core can take seconds (15 s on the real Travis road graph) to build.
$("sim-ms").textContent = `${sim.label} · loading…`;
sim.ready().then(
  () => ($("sim-ms").textContent = `${sim.label} · ready`),
  (e) => ($("sim-ms").textContent = `${sim.label} failed to start: ${e}`),
);

// ---- Desire lines (F06) -------------------------------------------------------
const DESIRE_N = 300; // top zone pairs drawn; more turns the map into a hairball
let showDesire = false;
async function refreshDesire(): Promise<void> {
  if (!showDesire) return setData(map, "desire", []);
  const lines = await queued(() => sim.desireLines(DESIRE_N)).catch((e) => {
    console.error(e);
    return [];
  });
  const max = Math.max(1, ...lines.map((d) => d.trips));
  setData(
    map,
    "desire",
    lines.map((d) => ({
      type: "Feature",
      // rel: share of the busiest pair, so widths read the same on any scenario size
      properties: { rel: d.trips / max, transitShare: d.trips ? d.transitTrips / d.trips : 0 },
      geometry: { type: "LineString", coordinates: [d.from, d.to] },
    })),
  );
}
$("btn-desire").onclick = () => {
  showDesire = !showDesire;
  $("btn-desire").classList.toggle("on", showDesire);
  void refreshDesire();
};

// Zoom re-renders only the map: the schematic morphs, the clock is untouched.
map.on("zoom", render);

// ---- UI wiring --------------------------------------------------------------
const drawer = new LineDrawer(
  map,
  (mode, pts) => addRoute(mode, pts),
  (drawing) => ($("draw-hint").hidden = !drawing),
);
$("btn-draw-bus").onclick = () => (editor.stop(), drawer.start("bus"));
$("btn-draw-rail").onclick = () => (editor.stop(), drawer.start("light_rail"));

const editor = new StopEditor(
  map,
  () => routes,
  render,
  (route) => void apply(() => sim.upsertRoute(route)),
  () => {
    updateEditHint();
    render();
    renderPanel();
  },
);
function updateEditHint(): void {
  const route = editor.route;
  $("draw-hint").hidden = !route;
  $("draw-hint").textContent = route
    ? `Editing ${route.name}: drag a stop to move it, click the map to add one, right-click a stop to delete it. Esc to finish.`
    : "Click to place stops. Enter or double-click to finish, Esc to cancel.";
}
$("btn-demo").onclick = () =>
  addRoute("commuter_rail", RED_LINE.map((s) => s.lngLat), RED_LINE.map((s) => s.name), "Red Line (demo)");

$("route-list").addEventListener("change", (e) => {
  const el = e.target as HTMLSelectElement;
  const r = routes.find((x) => x.id === el.closest<HTMLElement>(".route")?.dataset.id);
  if (r && el.dataset.act === "headway") {
    r.headwayMin = Number(el.value);
    void apply(() => sim.upsertRoute(r));
  }
});
$("route-list").addEventListener("click", (e) => {
  const el = e.target as HTMLElement;
  const id = el.closest<HTMLElement>(".route")?.dataset.id;
  const route = routes.find((x) => x.id === id);
  if (route && el.dataset.act === "edit") {
    if (editor.route?.id === id) editor.stop();
    else {
      drawer.cancel();
      editor.start(route);
    }
    return;
  }
  if (id && el.dataset.act === "del") {
    if (editor.route?.id === id) editor.stop();
    routes.splice(routes.findIndex((x) => x.id === id), 1);
    void apply(() => sim.removeRoute(id));
  }
});
// Rename: double-click a line name, Enter or click away to commit, Esc to cancel.
$("route-list").addEventListener("dblclick", (e) => {
  const el = e.target as HTMLElement;
  if (el.dataset.act !== "rename") return;
  const route = routes.find((x) => x.id === el.closest<HTMLElement>(".route")?.dataset.id);
  if (!route) return;
  el.contentEditable = "true";
  el.focus();
  document.getSelection()?.selectAllChildren(el);
  const finish = (commit: boolean) => {
    el.onblur = el.onkeydown = null;
    const name = el.textContent?.trim();
    if (commit && name && name !== route.name) {
      route.name = name;
      updateEditHint();
      void apply(() => sim.upsertRoute(route));
    } else renderPanel();
  };
  el.onblur = () => finish(true);
  el.onkeydown = (k) => {
    k.stopPropagation(); // keep Space/T from reaching the clock shortcuts
    if (k.key === "Enter") (k.preventDefault(), finish(true));
    if (k.key === "Escape") finish(false);
  };
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
  // Skip form controls: a focused button would also fire its own click on Space.
  if (drawer.active || e.target instanceof HTMLSelectElement || e.target instanceof HTMLButtonElement) return;
  if ((e.target as HTMLElement).isContentEditable) return;
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
