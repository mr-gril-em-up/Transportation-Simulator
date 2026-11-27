//! Transpo-Sim data pipeline: public Census and OpenStreetMap data in,
//! game-ready zone, demand and network files out.

pub mod acs;
pub mod aggregate;
pub mod census;
pub mod gtfs;
pub mod lines;
pub mod model;
pub mod osm;
pub mod scenario;

use anyhow::{Context, Result};
use census::Area;
use flate2::read::GzDecoder;
use model::*;
use serde::Serialize;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};

pub struct Config {
    /// Directory holding the files `scripts/fetch.sh` downloads.
    pub raw: PathBuf,
    pub out: PathBuf,
    pub area: Area,
    /// LODES data year.
    pub year: u32,
    /// Degrees added around the zones' bbox when clipping OSM.
    pub margin_deg: f64,
    /// Zone level of the outputs: block group (default), tract or county.
    pub level: aggregate::Level,
    /// Put today's network into `scenario.json` `lines` (DATA-7). It is always
    /// written to `existing_lines.json`.
    pub existing_lines: bool,
    /// Keep only motorway through secondary roads (`--roads highway`, DATA-8).
    pub highway_only: bool,
}

/// Summary written next to the data: counts, totals and the M0 exit-test
/// table (top commute flows), so a build can be checked without loading it.
#[derive(Serialize)]
pub struct Manifest {
    pub area: Vec<String>,
    pub lodes_year: u32,
    pub sources: Vec<&'static str>,
    pub zones: usize,
    pub population: u64,
    pub resident_workers: u64,
    pub jobs: u64,
    pub flows: usize,
    pub internal_commuters: u64,
    pub commute_out: u64,
    pub commute_in: u64,
    pub top_flows: Vec<TopFlow>,
    pub bbox: Option<osm::BBox>,
    pub road_nodes: Option<usize>,
    pub road_edges: Option<usize>,
    pub road_km: Option<f64>,
    pub transit_routes: Option<usize>,
    /// Road nodes and links in scenario.json after routing simplification
    /// (`scenario::simplify`); smaller than `road_nodes`/`road_edges`.
    pub scenario_road_nodes: usize,
    pub scenario_road_links: usize,
    /// Non-bus routes as "mode | ref | name | operator | stops", one per OSM
    /// relation (usually one per direction). Lets a build be checked for known
    /// lines, e.g. CapMetro's Red Line, from the manifest alone.
    pub rail_routes: Option<Vec<String>>,
    /// Zones with their own ACS median income / zero-car share (the rest use
    /// the tract or county fallback), and zones with a TIGER land area.
    /// `None` when the ACS or TIGER file was not present.
    pub acs_income_zones: Option<usize>,
    pub acs_zero_car_zones: Option<usize>,
    pub area_zones: Option<usize>,
    /// Area-wide ACS commute mode shares (B08301), for calibration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commute_mode_shares: Option<acs::CommuteShares>,
    /// Lines of today's network in `existing_lines.json` (and in
    /// `scenario.json` `lines` with `--existing-lines`).
    pub existing_lines: usize,
    /// OSM transit routes given GTFS headways (`null` without a GTFS feed).
    pub gtfs_routes_matched: Option<usize>,
    /// GTFS feeds read, with their route counts (DATA-9).
    pub gtfs_feeds: Vec<String>,
    /// `all` or `highway` (`--roads`).
    pub road_classes: &'static str,
    /// `block_group`, `tract` or `county`.
    pub zone_level: &'static str,
    /// Block groups merged into `zones` (equals `zones` at block group level).
    pub block_groups: usize,
}

#[derive(Serialize)]
pub struct TopFlow {
    pub home: String,
    pub work: String,
    pub workers: u32,
}

fn open(path: &Path) -> Result<Box<dyn Read>> {
    let f = BufReader::new(File::open(path).with_context(|| format!("open {}", path.display()))?);
    Ok(if path.extension().is_some_and(|e| e == "gz") {
        Box::new(GzDecoder::new(f))
    } else {
        Box::new(f)
    })
}

fn write_json<T: Serialize>(dir: &Path, name: &str, v: &T) -> Result<()> {
    let w = BufWriter::new(File::create(dir.join(name))?);
    serde_json::to_writer(w, v)?;
    Ok(())
}

pub fn build(cfg: &Config) -> Result<Manifest> {
    let raw = &cfg.raw;
    let y = cfg.year;
    let mut zones = census::read_zones(open(&raw.join("CenPop2020_Mean_BG48.txt"))?, &cfg.area)?;
    anyhow::ensure!(
        !zones.is_empty(),
        "no block groups match area {:?}",
        cfg.area.prefixes
    );
    let idx = census::index_by_geoid(&zones);
    census::add_lodes_totals(
        open(&raw.join(format!("tx_rac_S000_JT00_{y}.csv.gz")))?,
        "h_geocode",
        &mut zones,
        &idx,
        true,
    )?;
    census::add_lodes_totals(
        open(&raw.join(format!("tx_wac_S000_JT00_{y}.csv.gz")))?,
        "w_geocode",
        &mut zones,
        &idx,
        false,
    )?;
    let flows = census::read_flows(
        open(&raw.join(format!("tx_od_main_JT00_{y}.csv.gz")))?,
        &mut zones,
        &idx,
    )?;

    std::fs::create_dir_all(&cfg.out)?;
    let sum = |f: fn(&Zone) -> u32| zones.iter().map(|z| f(z) as u64).sum::<u64>();
    let mut m = Manifest {
        area: cfg.area.prefixes.clone(),
        lodes_year: y,
        sources: vec![
            "US Census Bureau, 2020 Census Centers of Population by block group",
            "US Census Bureau, LEHD Origin-Destination Employment Statistics (LODES) 8",
            "OpenStreetMap contributors (ODbL), via Geofabrik Texas extract",
        ],
        zones: zones.len(),
        population: sum(|z| z.population),
        resident_workers: sum(|z| z.resident_workers),
        jobs: sum(|z| z.jobs),
        flows: flows.len(),
        internal_commuters: flows.iter().map(|f| f.workers as u64).sum(),
        commute_out: sum(|z| z.commute_out),
        commute_in: sum(|z| z.commute_in),
        top_flows: top_flows(&zones, &flows),
        bbox: None,
        road_nodes: None,
        road_edges: None,
        road_km: None,
        transit_routes: None,
        scenario_road_nodes: 0,
        scenario_road_links: 0,
        rail_routes: None,
        acs_income_zones: None,
        acs_zero_car_zones: None,
        area_zones: None,
        commute_mode_shares: None,
        gtfs_routes_matched: None,
        gtfs_feeds: Vec::new(),
        existing_lines: 0,
        road_classes: if cfg.highway_only { "highway" } else { "all" },
        zone_level: cfg.level.name(),
        block_groups: zones.len(),
    };

    // ACS 5-year ending in the LODES year, and TIGER 2020 land area.
    let (inc_path, veh_path) = (
        raw.join(format!("acs5_bg48_{y}_b19013.dat")),
        raw.join(format!("acs5_bg48_{y}_b25044.dat")),
    );
    let acs = if inc_path.exists() && veh_path.exists() {
        m.sources
            .push("US Census Bureau, American Community Survey 5-year estimates (B19013, B25044)");
        Some(acs::read_acs(
            &std::fs::read_to_string(&inc_path)?,
            &std::fs::read_to_string(&veh_path)?,
        )?)
    } else {
        eprintln!(
            "note: {} or {} not found, zones keep default income and zero-car share",
            inc_path.display(),
            veh_path.display()
        );
        None
    };
    let dbf_path = raw.join("tl_2020_48_bg.dbf");
    let aland = if dbf_path.exists() {
        m.sources
            .push("US Census Bureau, TIGER/Line 2020 block groups (ALAND)");
        Some(acs::read_dbf_aland(&std::fs::read(&dbf_path)?)?)
    } else {
        eprintln!(
            "note: {} not found, zones have no land area",
            dbf_path.display()
        );
        None
    };
    let (inc, zc, ar) = acs::apply(&mut zones, acs.as_ref(), aland.as_ref());
    if acs.is_some() {
        m.acs_income_zones = Some(inc);
        m.acs_zero_car_zones = Some(zc);
    }
    if aland.is_some() {
        m.area_zones = Some(ar);
    }
    let b08301 = raw.join(format!("acs5_bg48_{y}_b08301.dat"));
    if b08301.exists() {
        m.commute_mode_shares = acs::commute_shares(
            &std::fs::read_to_string(&b08301)?,
            zones.iter().map(|z| z.geoid.as_str()),
        )?;
        if m.commute_mode_shares.is_some() {
            m.sources
                .push("US Census Bureau, American Community Survey 5-year estimates (B08301)");
        }
    }
    let (zones, flows) = aggregate::aggregate(&zones, &flows, cfg.level);
    if cfg.level != aggregate::Level::BlockGroup {
        m.zones = zones.len();
        m.flows = flows.len();
        m.top_flows = top_flows(&zones, &flows);
    }

    let pbf = raw.join("texas-latest.osm.pbf");
    let mut roads = None;
    let mut existing = Vec::new();
    if pbf.exists() {
        let mg = cfg.margin_deg;
        let bbox = zones
            .iter()
            .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, z| {
                [
                    b[0].min(z.lon - mg),
                    b[1].min(z.lat - mg),
                    b[2].max(z.lon + mg),
                    b[3].max(z.lat + mg),
                ]
            });
        let (r, mut transit) = osm::read_osm(&pbf, &bbox, cfg.highway_only)?;
        let mut feeds = Vec::new();
        for feed in gtfs::feeds() {
            let dir = raw.join("gtfs").join(&feed.id);
            if !dir.join("routes.txt").exists() {
                continue;
            }
            let hw = match gtfs::read_headways(&dir) {
                Ok(hw) => hw,
                Err(e) => {
                    eprintln!("warning: {} GTFS unreadable, skipped: {e:#}", feed.agency);
                    continue;
                }
            };
            let mut keys: Vec<_> = hw.iter().collect();
            keys.sort_by(|a, b| a.0.cmp(b.0));
            for (k, h) in keys {
                eprintln!(
                    "gtfs {} {k}: type {:?} peak {:?} midday {:?} span {} stops {}",
                    feed.id,
                    h.route_type,
                    h.peak_headway_min,
                    h.midday_headway_min,
                    h.span_hours,
                    h.stops.len()
                );
            }
            if feed.id == "capmetro" {
                m.gtfs_routes_matched = Some(gtfs::apply(&mut transit, &hw));
            }
            m.gtfs_feeds
                .push(format!("{} ({} routes)", feed.agency, hw.len()));
            feeds.push((feed, hw));
        }
        if !feeds.is_empty() {
            m.sources
                .push("Agency GTFS static feeds listed in reference/gtfs_feeds.csv");
        }
        m.bbox = Some(bbox);
        m.road_nodes = Some(r.nodes.len());
        m.road_edges = Some(r.edges.len());
        m.road_km =
            Some((r.edges.iter().map(|e| e.length_m as f64).sum::<f64>() / 100.0).round() / 10.0);
        m.transit_routes = Some(transit.routes.len());
        m.rail_routes = Some(rail_summary(&transit));
        write_json(&cfg.out, "roads.json", &r)?;
        existing = lines::existing_lines(&feeds, &bbox);
        write_json(&cfg.out, "existing_lines.json", &existing)?;
        write_json(&cfg.out, "transit.json", &transit)?;
        roads = Some(r);
    } else {
        eprintln!(
            "note: {} not found, skipping roads and transit",
            pbf.display()
        );
    }

    let mut scenario = scenario::to_scenario(&zones, &flows, roads.as_ref());
    m.existing_lines = existing.len();
    if cfg.existing_lines {
        scenario.lines = existing;
    }
    m.scenario_road_nodes = scenario.road_nodes.len();
    m.scenario_road_links = scenario.road_links.len();
    scenario
        .validate()
        .map_err(|e| anyhow::anyhow!("scenario failed sim-core validation: {e}"))?;
    write_json(&cfg.out, "scenario.json", &scenario)?;
    write_json(&cfg.out, "zones.json", &Zones { zones })?;
    let w = BufWriter::new(File::create(cfg.out.join("manifest.json"))?);
    serde_json::to_writer_pretty(w, &m)?;
    Ok(m)
}

/// The 20 largest home-to-work flows, for the manifest's exit-test table.
fn top_flows(zones: &[Zone], flows: &[Flow]) -> Vec<TopFlow> {
    let mut top: Vec<&Flow> = flows.iter().collect();
    top.sort_by_key(|f| (std::cmp::Reverse(f.workers), f.home, f.work));
    top.iter()
        .take(20)
        .map(|f| TopFlow {
            home: zones[f.home as usize].geoid.clone(),
            work: zones[f.work as usize].geoid.clone(),
            workers: f.workers,
        })
        .collect()
}

fn rail_summary(t: &Transit) -> Vec<String> {
    let dash = |o: &Option<String>| o.clone().unwrap_or_else(|| "-".into());
    let mut v: Vec<String> = t
        .routes
        .iter()
        .filter(|r| !matches!(r.mode.as_str(), "bus" | "trolleybus"))
        .map(|r| {
            let hw = match (r.peak_headway_min, r.midday_headway_min) {
                (None, None) => String::new(),
                (p, m) => format!(
                    " | headway {}/{} min",
                    p.map_or("-".into(), |x| x.to_string()),
                    m.map_or("-".into(), |x| x.to_string())
                ),
            };
            format!(
                "{} | {} | {} | {} | {} stops{hw}",
                r.mode,
                dash(&r.r#ref),
                dash(&r.name),
                dash(&r.operator),
                r.stops.len()
            )
        })
        .collect();
    v.sort();
    v
}
