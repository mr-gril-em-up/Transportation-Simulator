import "maplibre-gl/dist/maplibre-gl.css";
import "./style.css";
import { GameClock } from "./clock";
import { RED_LINE, TEXAS_BOUNDS, TRAVIS_CENTER } from "./data/geo";
import { LineDrawer } from "./draw";
import { StopEditor } from "./edit";
import { createMap, GEO_FULL, SCHEMATIC_FULL, setData, zoomBand } from "./map";
import { layoutSchematic, morph } from "./schematic";
import { km } from "./geo";
import { money } from "./format";
import { isRegional, type PendingRoute, type Budget, type ClockState, type LngLat, type Mode, type RouteResult, type RouteSpec } from "./sim/api";
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
let pending = new Map<string, PendingRoute>(); // routes under construction (F24)
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
    renderBudget(await sim.budget());
    pending = new Map((await sim.pendingRoutes()).map((p) => [p.routeId, p]));
    $("sim-ms").textContent = `${sim.label} · ${Math.round(performance.now() - t0)} ms`;
  } catch (e) {
    $("sim-ms").textContent = `${sim.label} error: ${e}`;
    console.error(e);
  }
  render();
  renderPanel();
}

let lastCash: number | null = null;
function renderBudget(b: Budget): void {
  // The change since the last call shows what the edit just cost (or the grant it won).
  const delta = lastCash === null ? 0 : b.cash - lastCash;
  lastCash = b.cash;
  $("budget").innerHTML =
    `Cash <b class="${b.cash < 0 ? "neg" : ""}">${money(b.cash)}</b>` +
    (Math.abs(delta) >= 1 ? ` (${delta > 0 ? "+" : ""}${money(delta)} last change)` : "") +
    `<br>This month: ${money(b.capitalSpentThisMonth)} built, ${money(b.grantsThisMonth)} federal grants`;
}

function makeRoute(id: string, mode: Mode, pts: LngLat[], stopNames?: string[], name?: string): RouteSpec {
  const n = routes.filter((r) => isRegional(r.mode) === isRegional(mode)).length + 1;
  return {
    id,
    name: name ?? (isRegional(mode) ? `Rail ${n}` : `Bus ${n}`),
    mode,
    color: COLORS[(nextId - 2) % COLORS.length],
    stops: pts.map((p, i) => ({ id: `${id}-s${i}`, name: stopNames?.[i] ?? `Stop ${i + 1}`, lngLat: p })),
    headwayMin: isRegional(mode) ? 30 : 15,
    fare: 1.25,
  };
}

function addRoute(mode: Mode, pts: LngLat[], stopNames?: string[], name?: string): void {
  const route = makeRoute(`r${nextId++}`, mode, pts, stopNames, name);
  routes.push(route);
  void apply(() => sim.upsertRoute(route));
}

// ---- Rendering --------------------------------------------------------------
/** New line not open yet: drawn faded. A rebuild draws normally (the old version runs). */
const building = (id: string) => pending.has(id) && !pending.get(id)!.replacesOpenLine;
/** During a rebuild the results are the running version's, whose stops differ from the drawn ones. */
const stopBoardings = (routeId: string, stopId: string) =>
  pending.has(routeId) ? 0 : (results.get(routeId)?.boardingsByStop[stopId] ?? 0);
/** "Day 184 (in 183 days)" for a sim-core day number. */
const openingLabel = (day: number) => `Day ${day + 1}` + (simDay >= 0 ? ` (in ${day - simDay} days)` : "");

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
      properties: { color: l.color, building: building(l.routeId) },
      geometry: { type: "LineString", coordinates: l.schem.map((s, i) => morph(s, l.geo[i], t)) },
    })),
  );
  for (const l of layouts)
    l.stops.forEach((s, idx) => {
      const b = stopBoardings(l.routeId, s.id);
      stops.push(stopFeature(morph(s.schem, s.geo, t), true, l.color, s.name, b, s.interchange, l.routeId, idx));
    });

  const bus = routes.filter((r) => !isRegional(r.mode));
  setData(
    map,
    "bus-lines",
    bus.map((r) => ({
      type: "Feature",
      properties: { color: r.color, building: building(r.id) },
      geometry: { type: "LineString", coordinates: r.stops.map((s) => s.lngLat) },
    })),
  );
  for (const r of bus)
    r.stops.forEach((s, idx) => {
      const b = stopBoardings(r.id, s.id);
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
      const p = pending.get(r.id);
      const build = p
        ? `<div class="econ building">${p.replacesOpenLine ? "Rebuild opens" : "Under construction, opens"} ${openingLabel(p.opensDay)}${p.replacesOpenLine ? "; current version keeps running" : ""}</div>`
        : "";
      const editing = editor.route?.id === r.id;
      return `<div class="route${editing ? " editing" : ""}" data-id="${r.id}">
        <span class="swatch" style="background:${r.color}"></span>
        <b class="name" data-act="rename" title="Double-click to rename">${escapeHtml(r.name)}</b> <span class="riders">${riders.toLocaleString()}</span> riders/day${econ}${build}
        <div class="row"><select data-act="headway">${opts}</select>
        <button data-act="edit">${editing ? "Done" : "Edit stops"}</button>
        <button data-act="del" title="Delete line">✕</button></div></div>`;
    })
    .join("");
}

// Startup: sim-core takes a few seconds to build (longer on the real Travis road graph).
$("sim-ms").textContent = `${sim.label} · loading…`;
const showLoading = setInterval(() => ($("sim-ms").textContent = `${sim.label} · loading…`), 500);
sim.ready().then(
  async () => {
    clearInterval(showLoading);
    $("sim-ms").textContent = `${sim.label} · ready`;
    renderBudget(await sim.budget());
    // Hand the clock to the sim, carrying over pause and speed set while it loaded.
    simReady = true;
    syncClock(await sim.setClock({ paused: clock.paused, timelapse: clock.speed }));
  },
  (e) => {
    clearInterval(showLoading);
    $("sim-ms").textContent = `${sim.label} failed to start: ${e}`;
  },
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
const DRAW_HINT = "Click to place stops. Enter or double-click to finish, Esc to cancel.";
const drawer = new LineDrawer(
  map,
  (mode, pts) => addRoute(mode, pts),
  (drawing) => {
    $("draw-hint").hidden = !drawing;
    $("draw-hint").textContent = DRAW_HINT;
  },
  previewDraft,
);

// ---- Build cost preview while drawing (F16) ------------------------------------
// Each placed stop asks the sim what the line would cost and carry. Previews go
// through the same queue as edits; a newer stop makes older pending ones skip.
let draftGen = 0;
function previewDraft(mode: Mode, pts: LngLat[]): void {
  const gen = ++draftGen;
  if (pts.length < 2) return;
  const draft = makeRoute("draft", mode, pts);
  let len = 0;
  for (let i = 1; i < pts.length; i++) len += km(pts[i - 1], pts[i]);
  const head = `${pts.length} stops · ${len.toFixed(1)} km`;
  $("draw-hint").textContent = `${head} · estimating cost…`;
  void queued(async () => (gen === draftGen ? sim.previewRoute(draft) : null))
    .then((p) => {
      if (!p || gen !== draftGen) return;
      const grant =
        p.federalGrant > 0 ? `${money(p.federalGrant)} federal grant` : "no federal grant (over $600k per daily rider)";
      $("draw-hint").innerHTML =
        `${head} · <b>${money(p.capitalCost)}</b> to build, ${grant} · ` +
        `<b>${p.dailyRiders.toLocaleString()}</b> riders/day` +
        (p.operatingCostPerDay !== undefined ? ` · ${money(p.operatingCostPerDay)}/day to run` : "") +
        (p.opensDay !== undefined && p.opensDay > simDay ? ` · opens ${openingLabel(p.opensDay)}` : "") +
        `<br>Enter or double-click to build, Esc to cancel.`;
    })
    .catch((e) => {
      if (gen === draftGen) $("draw-hint").textContent = `Cost preview failed: ${e}`;
    });
}
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
    : DRAW_HINT;
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

// ---- Clock (F21) ---------------------------------------------------------------
// The sim owns time: the shell's worker advances sim-core's clock and re-runs the
// day at each day boundary. The client polls it, interpolates between polls,
// and re-reads results when the day changes. Controls apply locally at once,
// then the sim's answer overwrites them.
const clock = new GameClock();
const pauseBtn = $("btn-pause");
const lapseBtn = $("btn-lapse");
const CLOCK_POLL_MS = 500;
let simDay = -1;
let simReady = false;

function syncClock(s: ClockState): void {
  clock.sync(s);
  pauseBtn.textContent = clock.paused ? "▶" : "⏸";
  lapseBtn.textContent = `Time-lapse ${clock.speed}x`;
  if (simDay >= 0 && s.day !== simDay) void apply(() => sim.results()); // the day re-ran
  simDay = s.day;
}
const clockCall = (p: Promise<ClockState>) => p.then(syncClock, (e) => console.error(e));
const togglePause = () => {
  clock.paused = !clock.paused;
  pauseBtn.textContent = clock.paused ? "▶" : "⏸";
  if (simReady) void clockCall(sim.setClock({ paused: clock.paused }));
};
const cycleLapse = () => {
  lapseBtn.textContent = `Time-lapse ${clock.cycleLapse()}x`;
  if (simReady) void clockCall(sim.setClock({ timelapse: clock.speed }));
};
pauseBtn.onclick = togglePause;
lapseBtn.onclick = cycleLapse;
$("btn-skip").onclick = () => {
  if (!simReady) return;
  void apply(async () => {
    const s = await sim.skipToNextMonth();
    simDay = s.day; // results are re-read below; keep the poll from fetching them again
    clock.sync(s);
    return sim.results();
  });
};

let polling = false;
setInterval(() => {
  if (!simReady || polling) return;
  polling = true;
  void clockCall(sim.clock()).finally(() => (polling = false));
}, CLOCK_POLL_MS);
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
