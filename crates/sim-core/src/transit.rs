//! Transit network: platforms (one per line and stop), ride and transfer
//! edges, zone access, path search (skims) and passenger loading.

use crate::geo::{dist_km, PointGrid};
use crate::graph::NONE;
use crate::params::{mode_defaults, ModeDefaults, Params};
use crate::roads::{runs_on_roads, Route};
use crate::schema::{Mode, TransitLine};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;

/// A zone as the transit network sees it.
#[derive(Clone, Copy)]
pub struct ZonePt {
    pub lon: f64,
    pub lat: f64,
    /// Radius of a disk with the zone's area, km.
    pub radius_km: f64,
}

pub struct LineMeta {
    /// Stable id the app uses (`LineId`).
    pub id: u32,
    pub name: String,
    pub mode: Mode,
    pub defaults: ModeDefaults,
    pub headway: f64,
    pub offpeak_headway: f64,
    pub fare: f64,
    pub fare_per_km: f64,
    pub span_hours: f64,
    pub first_platform: u32,
    pub seg_km: Vec<f64>,
    pub seg_min: Vec<f64>,
    /// Street geometry per segment for road-routed lines ([lon, lat] from
    /// stop to stop); empty for a segment that fell back to `alignment_factor`
    /// and for every segment of non-road modes.
    pub seg_shape: Vec<Vec<[f64; 2]>>,
}

impl LineMeta {
    /// One-way end-to-end running time, minutes.
    pub fn run_min(&self) -> f64 {
        self.seg_min.iter().sum()
    }
    pub fn length_km(&self) -> f64 {
        self.seg_km.iter().sum()
    }
}

pub struct Platform {
    pub line: u32,
    pub lon: f64,
    pub lat: f64,
    pub park_ride: bool,
    /// Parking spaces for this line's riders (infinite if unlimited).
    pub pnr_spaces: f64,
    /// Zone whose centroid is nearest (used for park-and-ride drive times).
    pub zone_near: u32,
}

#[derive(Clone, Copy)]
pub enum TEdge {
    Ride {
        to: u32,
        min: f64,
        km: f64,
        seg: u32,
        dir: u8,
    },
    Transfer {
        to: u32,
        walk_km: f64,
    },
}

pub struct TransitNet {
    pub lines: Vec<LineMeta>,
    pub platforms: Vec<Platform>,
    pub adj: Vec<Vec<TEdge>>,
    /// Per zone: platforms in walking range and the walk distance (km, on-street).
    pub walk_access: Vec<Vec<(u32, f64)>>,
    /// Per platform: zones in walking range and the walk distance.
    pub egress: Vec<Vec<(u32, f64)>>,
    pub pnr: Vec<u32>,
    /// Per platform: generalized minutes added to park-and-ride access when
    /// the lot is full (shadow price, SIM-11). Set by `Sim`.
    pub pnr_penalty: Vec<f64>,
    /// Per zone: share of the zone within walking range of a stop (0..1).
    pub coverage: Vec<f64>,
    /// Per platform: its connected component over ride and transfer edges.
    /// A search never leaves the components its seeds are in (SIM-21).
    pub comp: Vec<u32>,
}

/// Share of a disk (radius `r`) lying within `big_r` of a point `d` from its center.
/// Piecewise-linear approximation of the circle-overlap area.
pub fn coverage(d: f64, r: f64, big_r: f64) -> f64 {
    if r <= 1e-9 {
        return if d <= big_r { 1.0 } else { 0.0 };
    }
    if d >= r + big_r {
        return 0.0;
    }
    if big_r >= r {
        if d + r <= big_r {
            1.0
        } else {
            ((r + big_r - d) / (2.0 * r)).clamp(0.0, 1.0)
        }
    } else {
        let full = (big_r / r).powi(2);
        if d <= r - big_r {
            full
        } else {
            full * ((r + big_r - d) / (2.0 * big_r)).clamp(0.0, 1.0)
        }
    }
}

/// Connected components of the platform graph (edges are symmetric: every
/// ride has its reverse and transfers are found from both ends), numbered by
/// first platform.
fn components(adj: &[Vec<TEdge>]) -> Vec<u32> {
    let mut comp = vec![u32::MAX; adj.len()];
    let mut stack = Vec::new();
    let mut n = 0;
    for s in 0..adj.len() {
        if comp[s] != u32::MAX {
            continue;
        }
        comp[s] = n;
        stack.push(s);
        while let Some(v) = stack.pop() {
            for e in &adj[v] {
                let to = match *e {
                    TEdge::Ride { to, .. } | TEdge::Transfer { to, .. } => to as usize,
                };
                if comp[to] == u32::MAX {
                    comp[to] = n;
                    stack.push(to);
                }
            }
        }
        n += 1;
    }
    comp
}

impl TransitNet {
    /// `lines` are (id, line) pairs of the active lines. `route` gives the
    /// road route between two stops of a road-running line, or None to use
    /// straight-line km x `alignment_factor` (SIM-8).
    pub fn build(
        lines: &[(u32, &TransitLine)],
        zones: &[ZonePt],
        p: &Params,
        route: &mut dyn FnMut([f64; 2], [f64; 2]) -> Option<Arc<Route>>,
    ) -> TransitNet {
        let mut metas = Vec::new();
        let mut platforms = Vec::new();
        let mut adj: Vec<Vec<TEdge>> = Vec::new();
        // Spatial buckets keep the stop/zone joins near-linear statewide;
        // results equal the all-pairs scans exactly (SIM-20).
        let zone_pts: Vec<(f64, f64)> = zones.iter().map(|z| (z.lon, z.lat)).collect();
        let zone_grid = PointGrid::new(zone_pts.iter().copied(), 2.0);
        let mut buf = Vec::new();
        let mut nearest_zone = |lon: f64, lat: f64| -> u32 {
            zone_grid
                .nearest(lon, lat, &zone_pts, &mut buf)
                .unwrap_or(0)
        };
        for (li, (id, l)) in lines.iter().enumerate() {
            let dflt = mode_defaults(l.mode);
            let speed = l.speed_kph.unwrap_or(dflt.speed_kph).max(1.0);
            let first = platforms.len() as u32;
            for s in &l.stops {
                platforms.push(Platform {
                    line: li as u32,
                    lon: s.lon,
                    lat: s.lat,
                    park_ride: s.park_ride,
                    pnr_spaces: s.park_ride_spaces.unwrap_or(f64::INFINITY),
                    zone_near: nearest_zone(s.lon, s.lat),
                });
                adj.push(Vec::new());
            }
            let mut seg_km = Vec::new();
            let mut seg_min = Vec::new();
            let mut seg_shape = Vec::new();
            let on_roads = runs_on_roads(l.mode);
            for k in 0..l.stops.len() - 1 {
                let (a, b) = (&l.stops[k], &l.stops[k + 1]);
                let routed = if on_roads {
                    route([a.lon, a.lat], [b.lon, b.lat])
                } else {
                    None
                };
                let km = match &routed {
                    Some(r) => r.km,
                    None => dist_km(a.lon, a.lat, b.lon, b.lat) * dflt.alignment_factor,
                };
                seg_shape.push(routed.map(|r| r.shape.clone()).unwrap_or_default());
                let min = km / speed * 60.0 + dflt.dwell_s / 60.0;
                seg_km.push(km);
                seg_min.push(min);
                let (pa, pb) = (first + k as u32, first + k as u32 + 1);
                adj[pa as usize].push(TEdge::Ride {
                    to: pb,
                    min,
                    km,
                    seg: k as u32,
                    dir: 0,
                });
                adj[pb as usize].push(TEdge::Ride {
                    to: pa,
                    min,
                    km,
                    seg: k as u32,
                    dir: 1,
                });
            }
            metas.push(LineMeta {
                id: *id,
                name: l.name.clone(),
                mode: l.mode,
                defaults: dflt,
                headway: l.headway_min,
                offpeak_headway: l.offpeak_headway_min.unwrap_or(l.headway_min),
                fare: l.fare.unwrap_or(dflt.fare),
                fare_per_km: l.fare_per_km.unwrap_or(dflt.fare_per_km),
                span_hours: l.span_hours.unwrap_or(dflt.span_hours),
                first_platform: first,
                seg_km,
                seg_min,
                seg_shape: if on_roads { seg_shape } else { Vec::new() },
            });
        }
        // Transfers between platforms of different lines within walking distance.
        let big_r = p.stop_walk_km;
        let grid = PointGrid::new(
            platforms.iter().map(|pl| (pl.lon, pl.lat)),
            p.transfer_walk_km.max(big_r).max(0.1),
        );
        let mut near = Vec::new();
        for a in 0..platforms.len() {
            grid.near(
                platforms[a].lon,
                platforms[a].lat,
                p.transfer_walk_km,
                &mut near,
            );
            for &b in &near {
                let b = b as usize;
                if platforms[a].line == platforms[b].line {
                    continue;
                }
                let d = dist_km(
                    platforms[a].lon,
                    platforms[a].lat,
                    platforms[b].lon,
                    platforms[b].lat,
                );
                if d <= p.transfer_walk_km {
                    adj[a].push(TEdge::Transfer {
                        to: b as u32,
                        walk_km: d * p.detour_factor,
                    });
                }
            }
        }
        // Zone access and egress. `coverage` is zero beyond radius + big_r.
        // Keep the two lists symmetric: SIM-21's scoped edits are exact only
        // because a zone's access platforms are the ones that egress to it.
        let mut walk_access = vec![Vec::new(); zones.len()];
        let mut egress = vec![Vec::new(); platforms.len()];
        let mut cov = vec![0.0f64; zones.len()];
        for (zi, z) in zones.iter().enumerate() {
            grid.near(z.lon, z.lat, z.radius_km.max(0.0) + big_r, &mut near);
            for &pi in &near {
                let pl = &platforms[pi as usize];
                let d = dist_km(z.lon, z.lat, pl.lon, pl.lat);
                let c = coverage(d, z.radius_km, big_r);
                if c > 0.0 {
                    let walk = d.clamp(0.5 * big_r, big_r) * p.detour_factor;
                    walk_access[zi].push((pi, walk));
                    egress[pi as usize].push((zi as u32, walk));
                    cov[zi] = cov[zi].max(c);
                }
            }
        }
        let pnr = (0..platforms.len() as u32)
            .filter(|&i| platforms[i as usize].park_ride)
            .collect();
        let comp = components(&adj);
        TransitNet {
            lines: metas,
            pnr_penalty: vec![0.0; platforms.len()],
            comp,
            platforms,
            adj,
            walk_access,
            egress,
            pnr,
            coverage: cov,
        }
    }

    /// Per component: a hash of everything a search inside it reads (line
    /// ids, stops, positions, service, fares, park-and-ride and its penalty),
    /// keyed by line id and stop number rather than platform index, so the
    /// same component in a net rebuilt after an unrelated edit hashes the
    /// same (SIM-21).
    pub fn comp_sigs(&self) -> Vec<u64> {
        let n = self.comp.iter().map(|&c| c as usize + 1).max().unwrap_or(0);
        let mut h = vec![0xcbf2_9ce4_8422_2325u64; n];
        let mix = |h: &mut u64, x: u64| {
            for b in x.to_le_bytes() {
                *h ^= b as u64;
                *h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        for (pi, pl) in self.platforms.iter().enumerate() {
            let h = &mut h[self.comp[pi] as usize];
            let l = &self.lines[pl.line as usize];
            mix(h, l.id as u64);
            mix(h, (pi as u32 - l.first_platform) as u64);
            for x in [pl.lon, pl.lat, pl.pnr_spaces, self.pnr_penalty[pi]] {
                mix(h, x.to_bits());
            }
            mix(h, pl.park_ride as u64);
            if pi as u32 == l.first_platform {
                for x in [l.headway, l.offpeak_headway, l.fare, l.fare_per_km] {
                    mix(h, x.to_bits());
                }
                for x in l.seg_min.iter().chain(&l.seg_km) {
                    mix(h, x.to_bits());
                }
            }
        }
        h
    }

    pub fn is_empty(&self) -> bool {
        self.platforms.is_empty()
    }

    /// True if some line runs a different headway off-peak, so off-peak
    /// paths need their own search.
    pub fn offpeak_differs(&self) -> bool {
        self.lines.iter().any(|l| l.offpeak_headway != l.headway)
    }
}

/// Best-path components to one destination. `gc` is in generalized minutes at
/// the path VOT; mode choice re-prices components with each zone's own VOT.
#[derive(Clone, Copy, Debug)]
pub struct TOd {
    pub gc: f32,
    pub ivt: f32,
    pub wait: f32,
    pub walk_min: f32,
    pub fare: f32,
    pub xfers: f32,
    /// Headway of the last line ridden (drives schedule delay for work trips).
    pub last_headway: f32,
    pub drive_min: f32,
    pub drive_cost: f32,
    /// Park-and-ride platform used (drive access only; `NONE` otherwise).
    pub pnr_plat: u32,
    /// Full-lot shadow price at that platform, generalized minutes (SIM-11).
    pub pnr_penalty: f32,
}

impl TOd {
    pub const NONE: TOd = TOd {
        gc: f32::INFINITY,
        ivt: 0.0,
        wait: 0.0,
        walk_min: 0.0,
        fare: 0.0,
        xfers: 0.0,
        last_headway: 0.0,
        drive_min: 0.0,
        drive_cost: 0.0,
        pnr_plat: NONE,
        pnr_penalty: 0.0,
    };
    pub fn ok(&self) -> bool {
        self.gc.is_finite()
    }
}

#[derive(Clone, Copy)]
pub struct Label {
    pub gc: f64,
    pub ivt: f64,
    pub wait: f64,
    pub walk_min: f64,
    pub fare: f64,
    pub max_base: f64,
    pub last_h: f64,
    pub drive_min: f64,
    pub drive_cost: f64,
    pub xfers: u32,
    pub pred: u32,
    pub pred_edge: u32,
    /// Park-and-ride platform where the path started (`NONE` for walk access).
    pub access: u32,
}

impl Label {
    const UNSET: Label = Label {
        gc: f64::INFINITY,
        ivt: 0.0,
        wait: 0.0,
        walk_min: 0.0,
        fare: 0.0,
        max_base: 0.0,
        last_h: 0.0,
        drive_min: 0.0,
        drive_cost: 0.0,
        xfers: 0,
        pred: NONE,
        pred_edge: NONE,
        access: NONE,
    };
}

#[derive(Clone, Copy, PartialEq)]
struct Item(f64, u32);
impl Eq for Item {}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Access mode for a search.
#[derive(Clone, Copy, PartialEq)]
pub enum Access {
    Walk,
    /// Drive to a park-and-ride; `car_min[z]`, `car_km[z]` are car skims from the origin.
    Drive,
}

pub struct Searcher<'a> {
    pub net: &'a TransitNet,
    pub p: &'a Params,
    pub labels: Vec<Label>,
    heap: BinaryHeap<Item>,
    vot_min: f64,
    /// Per destination zone: (gc, platform, egress walk minutes).
    pub best: Vec<(f64, u32, f64)>,
    /// Search with off-peak headways (non-work trips) instead of peak ones.
    pub offpeak: bool,
    /// Platforms and zones the last run set, so the next run resets only
    /// those instead of every platform and zone in the state (SIM-20).
    touched_plats: Vec<u32>,
    touched_zones: Vec<u32>,
}

impl<'a> Searcher<'a> {
    pub fn new(net: &'a TransitNet, p: &'a Params, n_zones: usize) -> Self {
        Searcher {
            net,
            p,
            labels: vec![Label::UNSET; net.platforms.len()],
            heap: BinaryHeap::new(),
            vot_min: p.path_vot_per_hour / 60.0,
            best: vec![(f64::INFINITY, NONE, 0.0); n_zones],
            offpeak: false,
            touched_plats: Vec::new(),
            touched_zones: Vec::new(),
        }
    }

    fn walk_min(&self, km: f64) -> f64 {
        km / self.p.walk_kph * 60.0
    }

    fn board(&self, mut l: Label, plat: u32, walk_km: f64) -> Label {
        let line = &self.net.lines[self.net.platforms[plat as usize].line as usize];
        let w = self.walk_min(walk_km);
        let h = if self.offpeak {
            line.offpeak_headway
        } else {
            line.headway
        };
        let wait = self.p.wait_min(h);
        let fare_add = (line.fare - l.max_base).max(0.0);
        l.gc += self.p.walk_weight * w + self.p.wait_weight * wait + fare_add / self.vot_min;
        l.walk_min += w;
        l.wait += wait;
        l.fare += fare_add;
        l.max_base = l.max_base.max(line.fare);
        l.last_h = h;
        l
    }

    /// Run a search from zone `origin`. Fills `labels` and `best`. Returns
    /// false when no platform is within access (every destination is then
    /// unreachable, and callers can skip reading them).
    pub fn run(&mut self, origin: usize, access: Access, car_min: &[f32], car_km: &[f32]) -> bool {
        let net = self.net;
        for &v in &self.touched_plats {
            self.labels[v as usize] = Label::UNSET;
        }
        for &z in &self.touched_zones {
            self.best[z as usize] = (f64::INFINITY, NONE, 0.0);
        }
        self.touched_plats.clear();
        self.touched_zones.clear();
        self.heap.clear();
        let seed = |s: &mut Self, plat: u32, l: Label| {
            if l.gc < s.labels[plat as usize].gc {
                if !s.labels[plat as usize].gc.is_finite() {
                    s.touched_plats.push(plat);
                }
                s.labels[plat as usize] = l;
                s.heap.push(Item(l.gc, plat));
            }
        };
        match access {
            Access::Walk => {
                for &(plat, wk) in &net.walk_access[origin] {
                    let l = self.board(
                        Label {
                            gc: 0.0,
                            ..Label::UNSET
                        },
                        plat,
                        wk,
                    );
                    seed(self, plat, l);
                }
            }
            Access::Drive => {
                for &plat in &net.pnr {
                    let z = net.platforms[plat as usize].zone_near as usize;
                    let dm = car_min[z] as f64 + self.p.pnr_park_min;
                    if !dm.is_finite() || dm > self.p.pnr_max_drive_min {
                        continue;
                    }
                    let cost = car_km[z] as f64 * self.p.car_cost_per_km;
                    let start = Label {
                        gc: dm + cost / self.vot_min + net.pnr_penalty[plat as usize],
                        drive_min: dm,
                        drive_cost: cost,
                        access: plat,
                        ..Label::UNSET
                    };
                    let l = self.board(start, plat, 0.0);
                    seed(self, plat, l);
                }
            }
        }
        if self.heap.is_empty() {
            return false;
        }
        while let Some(Item(gc, v)) = self.heap.pop() {
            if gc > self.labels[v as usize].gc {
                continue;
            }
            let cur = self.labels[v as usize];
            for (ei, e) in net.adj[v as usize].iter().enumerate() {
                let (to, mut nl) = match *e {
                    TEdge::Ride { to, min, km, .. } => {
                        let line = &net.lines[net.platforms[v as usize].line as usize];
                        let f = line.fare_per_km * km;
                        let mut l = cur;
                        l.gc += min + f / self.vot_min;
                        l.ivt += min;
                        l.fare += f;
                        (to, l)
                    }
                    TEdge::Transfer { to, walk_km } => {
                        let mut l = self.board(cur, to, walk_km);
                        l.gc += self.p.transfer_penalty_min;
                        l.xfers += 1;
                        (to, l)
                    }
                };
                nl.pred = v;
                nl.pred_edge = ei as u32;
                if nl.gc < self.labels[to as usize].gc {
                    if !self.labels[to as usize].gc.is_finite() {
                        self.touched_plats.push(to);
                    }
                    self.labels[to as usize] = nl;
                    self.heap.push(Item(nl.gc, to));
                }
            }
        }
        // Destinations: egress walk from any platform reached by riding,
        // in platform order so ties resolve as a scan of all platforms would.
        self.touched_plats.sort_unstable();
        for &plat in &self.touched_plats {
            let l = &self.labels[plat as usize];
            if l.ivt <= 0.0 {
                continue;
            }
            for &(z, wk) in &net.egress[plat as usize] {
                let w = wk / self.p.walk_kph * 60.0;
                let g = l.gc + self.p.walk_weight * w;
                let b = &mut self.best[z as usize];
                if g < b.0 {
                    if !b.0.is_finite() {
                        self.touched_zones.push(z);
                    }
                    *b = (g, plat, w);
                }
            }
        }
        self.best[origin].0 = f64::INFINITY; // no intrazonal transit
        true
    }

    /// Path components to zone `dest` from the last `run`.
    pub fn od(&self, dest: usize) -> TOd {
        let (g, plat, w) = self.best[dest];
        if !g.is_finite() {
            return TOd::NONE;
        }
        let l = &self.labels[plat as usize];
        TOd {
            gc: g as f32,
            ivt: l.ivt as f32,
            wait: l.wait as f32,
            walk_min: (l.walk_min + w) as f32,
            fare: l.fare as f32,
            xfers: l.xfers as f32,
            last_headway: l.last_h as f32,
            drive_min: l.drive_min as f32,
            drive_cost: l.drive_cost as f32,
            pnr_plat: l.access,
            pnr_penalty: if l.access == NONE {
                0.0
            } else {
                self.net.pnr_penalty[l.access as usize] as f32
            },
        }
    }

    /// Add `trips` to `dest` along the best path into `acc`.
    pub fn load(&self, dest: usize, trips: f64, acc: &mut LineLoads) {
        let (g, mut plat, _) = self.best[dest];
        if !g.is_finite() || trips <= 0.0 {
            return;
        }
        loop {
            let l = &self.labels[plat as usize];
            let line = self.net.platforms[plat as usize].line as usize;
            let stop = (plat - self.net.lines[line].first_platform) as usize;
            if l.pred == NONE {
                acc.boardings[line] += trips;
                acc.stop_boardings[line][stop] += trips;
                break;
            }
            match self.net.adj[l.pred as usize][l.pred_edge as usize] {
                TEdge::Ride { km, seg, dir, .. } => {
                    acc.seg[line][seg as usize][dir as usize] += trips;
                    acc.pkm[line] += trips * km;
                }
                TEdge::Transfer { .. } => {
                    acc.boardings[line] += trips;
                    acc.stop_boardings[line][stop] += trips;
                }
            }
            plat = l.pred;
        }
    }
}

/// Passenger loads per line (indexed like `TransitNet::lines`).
#[derive(Clone, Default)]
pub struct LineLoads {
    pub boardings: Vec<f64>,
    /// Per line, boardings at each stop.
    pub stop_boardings: Vec<Vec<f64>>,
    pub pkm: Vec<f64>,
    /// Per segment, [forward, reverse] daily passengers.
    pub seg: Vec<Vec<[f64; 2]>>,
}

impl LineLoads {
    pub fn new(net: &TransitNet) -> Self {
        LineLoads {
            boardings: vec![0.0; net.lines.len()],
            stop_boardings: net
                .lines
                .iter()
                .map(|l| vec![0.0; l.seg_km.len() + 1])
                .collect(),
            pkm: vec![0.0; net.lines.len()],
            seg: net
                .lines
                .iter()
                .map(|l| vec![[0.0; 2]; l.seg_km.len()])
                .collect(),
        }
    }
    /// Replace the loads of lines `keep` maps to a line of `old` with that
    /// line's old loads (SIM-21).
    pub fn carry(&mut self, old: &LineLoads, keep: &[Option<usize>]) {
        for (i, k) in keep.iter().enumerate() {
            if let Some(o) = *k {
                self.boardings[i] = old.boardings[o];
                self.pkm[i] = old.pkm[o];
                self.stop_boardings[i].clone_from(&old.stop_boardings[o]);
                self.seg[i].clone_from(&old.seg[o]);
            }
        }
    }

    pub fn merge(mut self, o: LineLoads) -> Self {
        for i in 0..self.boardings.len() {
            self.boardings[i] += o.boardings[i];
            self.pkm[i] += o.pkm[i];
            for (a, b) in self.stop_boardings[i].iter_mut().zip(&o.stop_boardings[i]) {
                *a += b;
            }
            for (a, b) in self.seg[i].iter_mut().zip(&o.seg[i]) {
                a[0] += b[0];
                a[1] += b[1];
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_shapes() {
        assert_eq!(coverage(0.0, 0.3, 0.8), 1.0);
        assert_eq!(coverage(5.0, 0.3, 0.8), 0.0);
        let half = coverage(0.8, 0.3, 0.8);
        assert!(half > 0.4 && half < 0.6);
        // Big zone: a stop at its center covers (R/r)^2 of it.
        assert!((coverage(0.0, 1.6, 0.8) - 0.25).abs() < 1e-9);
    }
}
