import "maplibre-gl/dist/maplibre-gl.css";
import "./style.css";
import { GameClock } from "./clock";
import { RED_LINE, TEXAS_BOUNDS, TRAVIS_CENTER } from "./data/geo";
import { LineDrawer } from "./draw";
import { StopEditor } from "./edit";
import { createMap, GEO_FULL, SCHEMATIC_FULL, setData, zoomBand } from "./map";
import { layoutSchematic, morph } from "./schematic";
import { km, routePath } from "./geo";
import { money, escapeHtml } from "./format";
import { inspectorHtml } from "./inspector";
import { isRegional, defaultFare, type PendingRoute, type Budget, type Statement, type ClockState, type LngLat, type LoadProgress, type Mode, type RouteResult, type RouteSpec } from "./sim/api";
import { createSim } from "./sim";
import { countyAt, countyName, loadingText } from "./statewide";
import { serialQueue } from "./sim/queue";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const COLORS = ["#dc241f", "#0098d4", "#00782a", "#9b0056", "#f3a9bb", "#ee7c0e", "#6950a1", "#b36305"];

const sim = createSim();
const map = await createMap("map");
if (import.meta.env.DEV) Object.assign(window, { __map: map }); // for e2e scripts
const routes: RouteSpec[] = [];
/** Real rail and intercity lines sim-core does not run: drawn in the schematic, never sent to the sim. */
let reference: RouteSpec[] = [];
/** Zone centroids with county FIPS, for naming the county under the camera. */
let zoneCounty: [number, number, number][] = [];
let statewide = false;
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
let lastBudget: Budget | null = null;
function renderBudget(b: Budget): void {
  lastBudget = b;
  // The change since the last call shows what the edit just cost (or the grant it won).
  const delta = lastCash === null ? 0 : b.cash - lastCash;
  lastCash = b.cash;
  // Bond sizes offered: what covers the deficit (rounded up to $10M), then fixed steps.
  // Sizes above the borrowing capacity (SIM-7/SIM-9) would be refused, so they are not offered.
  const cap = b.bondCapacity;
  const cover = b.cash < 0 ? Math.min(Math.ceil(-b.cash / 1e7) * 1e7, Math.floor(cap / 1e6) * 1e6) : 0;
  const sizes = [...new Set([cover, 100e6, 500e6, 1e9].filter((x) => x > 0 && x <= cap))];
  const capText = Number.isFinite(cap) ? `Borrowing capacity ${money(cap)}, backed by ${money(b.taxCapacity)}/year of local tax` : "";
  const terms = `${(b.bondRate * 100).toFixed(2)}% over ${Math.round(b.bondTermMonths / 12)} years, payments start next month`;
  const status =
    b.bankruptMonth !== null
      ? `<div class="alert">Bankrupt since month ${b.bankruptMonth + 1}: the game is over. The sim keeps running so you can see why.</div>`
      : b.distressedMonths > 0
        ? `<div class="warn">${b.distressedMonths} of ${b.bankruptcyMonths} months in a row in distress (cash below zero or debt payments above the operating surplus). Reaching ${b.bankruptcyMonths} is bankruptcy.</div>`
        : b.cash < 0
          ? `<div class="warn">Cash is negative. Each month that ends below zero counts toward bankruptcy (${b.bankruptcyMonths} in a row).</div>`
          : "";
  $("budget").innerHTML =
    `Cash <b class="${b.cash < 0 ? "neg" : ""}">${money(b.cash)}</b>` +
    (Math.abs(delta) >= 1 ? ` (${delta > 0 ? "+" : ""}${money(delta)} last change)` : "") +
    `<br>This month: ${money(b.capitalSpentThisMonth)} built, ${money(b.grantsThisMonth)} federal grants` +
    (b.debt > 0 ? `<br>Debt ${money(b.debt)}, ${money(b.debtServicePerMonth)}/month in payments` : "") +
    status +
    (b.bankruptMonth !== null
      ? ""
      : sizes.length
        ? `<div class="row"><select id="bond-size" title="${terms}">${sizes
            .map((x) => `<option value="${x}">${money(x)}${x === cover ? (cover < -b.cash ? " (all you can borrow)" : " (covers deficit)") : ""}</option>`)
            .join("")}</select> <button id="btn-bond" title="${terms}">Issue bond</button></div>` +
          (capText ? `<small>${capText}</small>` : "")
        : `<div class="row"><small>No borrowing capacity: bonds are backed by the local tax around open and planned stops. Draw a line first.</small></div>`) +
    renderStatements(b);
}

/** Rows of the statement table: label, value, and whether it is a subtotal. */
const STATEMENT_ROWS: [string, (m: Statement) => number, boolean?][] = [
  ["Fares", (m) => m.fares],
  ["Operating assistance", (m) => m.operatingAssistance],
  ["Local sales tax", (m) => m.localTax],
  ["Operating cost", (m) => -m.operatingCost],
  ["Operating surplus", (m) => m.fares + m.operatingAssistance + m.localTax - m.operatingCost, true],
  ["Debt payments", (m) => -m.debtService],
  ["Capital spent", (m) => -m.capitalSpent],
  ["Federal capital grants", (m) => m.federalGrants],
  ["Bonds issued", (m) => m.bondProceeds],
  ["Change in cash", (m) => m.net, true],
];
const STATEMENT_COLS = 2; // three overflow the 260 px panel
let statementsOpen = false;

/** Monthly statements (F26): the last settled months side by side, newest first. */
function renderStatements(b: Budget): string {
  const cols = b.statements.slice(0, STATEMENT_COLS);
  const body = cols.length
    ? `<table><tr><th></th>${cols.map((m) => `<th>Month ${m.month + 1}</th>`).join("")}</tr>${STATEMENT_ROWS.map(
        ([label, f, total]) =>
          `<tr class="${total ? "total" : ""}"><td>${label}</td>${cols
            .map((m) => `<td class="${f(m) < 0 ? "neg" : ""}">${money(f(m))}</td>`)
            .join("")}</tr>`,
      ).join("")}</table>` +
      (cols[0].interest > 0 ? `<small>Interest in last month's payments: ${money(cols[0].interest)}</small>` : "") +
      (cols[0].capitalizedInterest > 0
        ? `<small>${cols[0].interest > 0 ? "<br>" : ""}Interest added to debt during construction last month (no cash): ${money(cols[0].capitalizedInterest)}</small>`
        : "")
    : `<small>No month has closed yet. Fares, operating cost and tax are booked when a month ends (Skip to next month to see one).</small>`;
  return `<details id="statements" ${statementsOpen ? "open" : ""}><summary>Monthly statements</summary>${body}</details>`;
}

function makeRoute(id: string, mode: Mode, pts: LngLat[], stopNames?: string[], name?: string): RouteSpec {
  const n = routes.filter((r) => !existing.has(r.id) && isRegional(r.mode) === isRegional(mode)).length + 1;
  return {
    id,
    name: name ?? (isRegional(mode) ? `Rail ${n}` : `Bus ${n}`),
    mode,
    color: COLORS[(nextId - 2) % COLORS.length],
    stops: pts.map((p, i) => ({ id: `${id}-s${i}`, name: stopNames?.[i] ?? `Stop ${i + 1}`, lngLat: p })),
    headwayMin: isRegional(mode) ? 30 : 15,
    fare: defaultFare(mode),
  };
}

function addRoute(mode: Mode, pts: LngLat[], stopNames?: string[], name?: string, service?: Partial<RouteSpec>): void {
  const route = { ...makeRoute(`r${nextId++}`, mode, pts, stopNames, name), ...service };
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

/**
 * Street path for a bus-like line (SIM-8). The line being edited draws straight so stops track the
 * cursor. A pending line uses the shape from its build preview, because sim-core's results describe
 * the running version (or nothing, before it first opens).
 */
function busPath(r: RouteSpec): LngLat[] {
  const pts = r.stops.map((s) => s.lngLat);
  if (editor.route?.id === r.id) return pts;
  const shape = pending.has(r.id) ? drawnShape.get(r.id) : (results.get(r.id)?.shape ?? drawnShape.get(r.id));
  return routePath(pts, shape);
}
/** Street shape from the last build preview of each drawn line, until sim-core reports its own. */
const drawnShape = new Map<string, LngLat[][]>();

function render(): void {
  const z = map.getZoom();
  // While editing, draw true geography so dragged stops land where the cursor is.
  const t = editor.active ? 1 : Math.min(1, Math.max(0, (z - SCHEMATIC_FULL) / (GEO_FULL - SCHEMATIC_FULL)));
  const pxPerDeg = (512 * 2 ** z) / 360;
  const rail = [...routes.filter((r) => isRegional(r.mode)), ...reference];
  // Fully geographic, every stop shows (the layout thins stops only for the schematic).
  const layouts = layoutSchematic(rail, t === 1 ? Infinity : pxPerDeg);
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
    l.stops.forEach((s) => {
      const b = stopBoardings(l.routeId, s.id);
      stops.push(stopFeature(morph(s.schem, s.geo, t), true, l.color, s.name, b, s.interchange, l.routeId, s.idx));
    });

  const bus = routes.filter((r) => !isRegional(r.mode));
  setData(
    map,
    "bus-lines",
    bus.map((r) => ({
      type: "Feature",
      properties: { color: r.color, building: building(r.id) },
      geometry: { type: "LineString", coordinates: busPath(r) },
    })),
  );
  for (const r of bus)
    r.stops.forEach((s, idx) => {
      const b = stopBoardings(r.id, s.id);
      stops.push(stopFeature(s.lngLat, false, r.color, s.name, b, false, r.id, idx));
    });
  setData(map, "stops", stops);

  // County detail (statewide): from the City band in, name the county under the camera. The
  // score does not change with the camera; this is the view handing off, not a second model.
  const c = map.getCenter();
  const fips = statewide && zoomBand(z) !== "State" && zoomBand(z) !== "Region" ? countyAt(zoneCounty, [c.lng, c.lat]) : null;
  $("zoom-badge").innerHTML =
    `<b>${zoomBand(z)}</b> z${z.toFixed(1)} · ${t === 0 ? "schematic" : t === 1 ? "geographic" : "morphing"}` +
    (fips ? ` · <b>${countyName(fips)}</b>` : "") +
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

/** Routes whose inspector is open (survives the panel's re-renders). */
const inspecting = new Set<string>();

/** Ids of lines present at start (today's network, F44); grouped apart in the panel. */
const existing = new Set<string>();
let existingOpen = false;
const EXISTING_COLOR = { rail: "#dc241f", bus: "#8a8a8a" };
let referenceOpen = false;

function renderPanel(): void {
  const list = $("route-list");
  if (!routes.length && !reference.length) {
    list.innerHTML = "<em>No lines yet. Draw one.</em>";
    return;
  }
  const own = routes.filter((r) => !existing.has(r.id));
  // Rail first, then busiest first: 60-odd rows are only scannable in that order.
  const ridersOf = (r: RouteSpec) => results.get(r.id)?.dailyRiders ?? 0;
  const start = routes
    .filter((r) => existing.has(r.id))
    .sort((a, b) => Number(isRegional(b.mode)) - Number(isRegional(a.mode)) || ridersOf(b) - ridersOf(a));
  const riders = start.reduce((t, r) => t + ridersOf(r), 0);
  list.innerHTML =
    (own.length ? own.map(routeRow).join("") : "<em>No lines of your own yet. Draw one.</em>") +
    (start.length
      ? `<details id="existing" ${existingOpen ? "open" : ""}><summary>Today's network: ${start.length} lines, ${riders.toLocaleString()} riders/day</summary>${start.map(routeRow).join("")}</details>`
      : "") +
    (reference.length
      ? `<details class="ref" id="reference" ${referenceOpen ? "open" : ""}><summary>Other real rail: ${reference.length} lines, drawn only (not simulated yet)</summary><ul>${reference
          .map((r) => `<li><span class="swatch" style="background:${r.color}"></span>${escapeHtml(r.name)}</li>`)
          .join("")}</ul></details>`
      : "");
}

function routeRow(r: RouteSpec): string {
  const res = results.get(r.id);
  const riders = res?.dailyRiders ?? 0;
  const econ =
    res?.operatingCostPerDay !== undefined
      ? `<div class="econ">$${Math.round(res.operatingCostPerDay).toLocaleString()}/day to run · farebox ${Math.round((res.fareboxRecovery ?? 0) * 100)}%</div>`
      : "";
  // The preset's 32 min is not a menu step; keep the current value selectable.
  const opts = [...new Set([5, 10, 15, 20, 30, 60, r.headwayMin])]
    .sort((a, b) => a - b)
    .map((h) => `<option value="${h}" ${h === r.headwayMin ? "selected" : ""}>every ${h} min</option>`)
    .join("");
  const offpeak =
    r.offpeakHeadwayMin !== undefined ? `<div class="econ">Peak headway below; every ${r.offpeakHeadwayMin} min off-peak</div>` : "";
  const p = pending.get(r.id);
  const build = p
    ? `<div class="econ building">${p.replacesOpenLine ? "Rebuild opens" : "Under construction, opens"} ${openingLabel(p.opensDay)}${p.replacesOpenLine ? "; current version keeps running" : ""}</div>`
    : "";
  const editing = editor.route?.id === r.id;
  // During a rebuild sim-core's per-stop figures follow the running version's stops, not the drawn ones.
  const canInspect = res?.detail !== undefined && !pending.has(r.id) && riders > 0;
  const inspect =
    canInspect && inspecting.has(r.id)
      ? inspectorHtml(
          r.stops.map((s) => ({ name: s.name, boardings: res.boardingsByStop[s.id] ?? 0 })),
          res.detail!,
        )
      : "";
  return `<div class="route${editing ? " editing" : ""}" data-id="${r.id}">
    <span class="swatch" style="background:${r.color}"></span>
    <b class="name" data-act="rename" title="Double-click to rename">${escapeHtml(r.name)}</b> <span class="riders">${riders.toLocaleString()}</span> riders/day${econ}${build}${offpeak}
    <div class="row"><select data-act="headway">${opts}</select>
    <button data-act="edit">${editing ? "Done" : "Edit stops"}</button>
    <button data-act="del" title="Delete line">✕</button></div>
    ${canInspect ? `<div class="toggle" data-act="inspect" title="Boardings, loads and capacity">${inspecting.has(r.id) ? "▼" : "▶"} Inspect line</div>` : ""}${inspect}</div>`;
}

// Startup: sim-core takes a few seconds to build (longer on the real Travis road graph).
$("sim-ms").textContent = `${sim.label} · loading…`;
const showLoading = setInterval(() => ($("sim-ms").textContent = `${sim.label} · loading…`), 500);
// Loading screen (statewide startup is over a minute): stage, bar, elapsed. Opens on the
// scenario's extent once it is read, so a statewide game starts on all of Texas.
let framed = false;
const onProgress = (p: LoadProgress) => {
  statewide = p.statewide;
  if (p.statewide && p.bounds && !framed) {
    framed = true;
    map.fitBounds(p.bounds, { padding: 40, duration: 0 });
  }
  if (p.stage === "ready") return;
  // A county build is ready in seconds; show the screen only for the statewide wait.
  $("loading").hidden = !p.statewide;
  const { stage, detail, fill } = loadingText(p);
  $("loading-stage").textContent = stage;
  $("loading-detail").textContent = detail;
  $("loading-fill").style.width = `${Math.round(fill * 100)}%`;
};
sim.ready(onProgress).then(
  async () => {
    clearInterval(showLoading);
    $("loading").hidden = true;
    $("sim-ms").textContent = `${sim.label} · ready`;
    renderBudget(await sim.budget());
    // Today's network when the shell started with it (the provisional default, APP-4; empty on a blank map).
    const start = await sim.existingRoutes();
    if (start.length) {
      for (const r of start) {
        r.color = isRegional(r.mode) ? EXISTING_COLOR.rail : EXISTING_COLOR.bus;
        existing.add(r.id);
        routes.push(r);
      }
      // The Red Line is already running; a second copy would compete with it.
      $("btn-demo").hidden = start.some((r) => r.mode === "commuter_rail");
      await apply(() => sim.results());
    }
    // Real rail and intercity lines the sim does not run, and counties for the badge.
    [reference, zoneCounty] = await Promise.all([sim.referenceLines(), sim.zoneCounties()]);
    renderPanel();
    render();
    // Hand the clock to the sim, carrying over pause and speed set while it loaded.
    simReady = true;
    syncClock(await sim.setClock({ paused: clock.paused, timelapse: clock.speed }));
  },
  (e) => {
    clearInterval(showLoading);
    $("loading").hidden = true;
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
map.on("moveend", render); // the county name follows panning too

// ---- UI wiring --------------------------------------------------------------
const DRAW_HINT = "Click to place stops. Enter or double-click to finish, Esc to cancel.";
const drawer = new LineDrawer(
  map,
  (mode, pts) => {
    const shape = draftShape && draftShape.pts.length === pts.length && draftShape.pts.every((p, i) => p === pts[i]) ? draftShape.shape : undefined;
    draftShape = null;
    addRoute(mode, pts);
    if (shape) drawnShape.set(`r${nextId - 1}`, shape);
  },
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
let draftShape: { pts: LngLat[]; shape: LngLat[][] } | null = null;
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
      if (p.shape) {
        draftShape = { pts, shape: p.shape };
        drawer.showPath(pts, routePath(pts, p.shape));
      }
      const road = p.lengthKm !== undefined && p.shape ? ` (${p.lengthKm.toFixed(1)} km by road)` : "";
      // Capital is charged even if it cannot be financed (SIM-9): compare with what can be raised now.
      const b = lastBudget;
      const net = p.capitalCost - p.federalGrant;
      const raise = b ? Math.max(0, b.cash) + (Number.isFinite(b.bondCapacity) ? b.bondCapacity : Infinity) : Infinity;
      const finance =
        b && net > raise
          ? `<br><span class="neg">${money(net)} after the grant is more than you can raise now: ${money(Math.max(0, b.cash))} cash + ${money(b.bondCapacity)} borrowing capacity. Building it adds its stops' tax, which backs more borrowing.</span>`
          : "";
      const grant =
        p.federalGrant > 0 ? `${money(p.federalGrant)} federal grant` : "no federal grant (over $600k per daily rider)";
      $("draw-hint").innerHTML =
        `${head}${road} · <b>${money(p.capitalCost)}</b> to build, ${grant} · ` +
        `<b>${p.dailyRiders.toLocaleString()}</b> riders/day` +
        (p.operatingCostPerDay !== undefined ? ` · ${money(p.operatingCostPerDay)}/day to run` : "") +
        (p.opensDay !== undefined && p.opensDay > simDay ? ` · opens ${openingLabel(p.opensDay)}` : "") +
        finance +
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
// Real MetroRail service: every 32 min in the peaks, 51 midday (sim-core SIM-3, figures.csv).
$("btn-demo").onclick = () =>
  addRoute("commuter_rail", RED_LINE.map((s) => s.lngLat), RED_LINE.map((s) => s.name), "Red Line (demo)", {
    headwayMin: 32,
    offpeakHeadwayMin: 51,
  });

// The budget panel re-renders every day; remember whether the statements are open.
// "toggle" does not bubble, so listen in the capture phase.
$("route-list").addEventListener("toggle", (e) => {
  if ((e.target as HTMLElement).id === "existing") existingOpen = (e.target as HTMLDetailsElement).open;
  if ((e.target as HTMLElement).id === "reference") referenceOpen = (e.target as HTMLDetailsElement).open;
}, true);
$("budget").addEventListener("toggle", (e) => {
  if ((e.target as HTMLElement).id === "statements") statementsOpen = (e.target as HTMLDetailsElement).open;
}, true);

// Bonds (F26): the budget panel re-renders, so delegate from its container.
$("budget").addEventListener("click", (e) => {
  if ((e.target as HTMLElement).id !== "btn-bond") return;
  const principal = Number(($("bond-size") as HTMLSelectElement).value);
  void queued(() => sim.issueBond(principal)).then(renderBudget, (err) => {
    $("sim-ms").textContent = `${sim.label} error: ${err}`;
  });
});

$("route-list").addEventListener("change", (e) => {
  const el = e.target as HTMLSelectElement;
  const r = routes.find((x) => x.id === el.closest<HTMLElement>(".route")?.dataset.id);
  if (r && el.dataset.act === "headway") {
    r.headwayMin = Number(el.value);
    delete r.offpeakHeadwayMin; // the menu sets one all-day headway
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
  if (id && el.dataset.act === "inspect") {
    if (!inspecting.delete(id)) inspecting.add(id);
    renderPanel();
    return;
  }
  if (id && el.dataset.act === "del") {
    inspecting.delete(id);
    existing.delete(id);
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
