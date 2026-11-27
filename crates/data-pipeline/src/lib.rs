//! Transpo-Sim data pipeline: public Census and OpenStreetMap data in,
//! game-ready zone, demand and network files out.

pub mod census;
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
    let mut top: Vec<&Flow> = flows.iter().collect();
    top.sort_by_key(|f| (std::cmp::Reverse(f.workers), f.home, f.work));
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
        top_flows: top
            .iter()
            .take(20)
            .map(|f| TopFlow {
                home: zones[f.home as usize].geoid.clone(),
                work: zones[f.work as usize].geoid.clone(),
                workers: f.workers,
            })
            .collect(),
        bbox: None,
        road_nodes: None,
        road_edges: None,
        road_km: None,
        transit_routes: None,
        scenario_road_nodes: 0,
        scenario_road_links: 0,
        rail_routes: None,
    };

    let pbf = raw.join("texas-latest.osm.pbf");
    let mut roads = None;
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
        let (r, transit) = osm::read_osm(&pbf, &bbox)?;
        m.bbox = Some(bbox);
        m.road_nodes = Some(r.nodes.len());
        m.road_edges = Some(r.edges.len());
        m.road_km =
            Some((r.edges.iter().map(|e| e.length_m as f64).sum::<f64>() / 100.0).round() / 10.0);
        m.transit_routes = Some(transit.routes.len());
        m.rail_routes = Some(rail_summary(&transit));
        write_json(&cfg.out, "roads.json", &r)?;
        write_json(&cfg.out, "transit.json", &transit)?;
        roads = Some(r);
    } else {
        eprintln!(
            "note: {} not found, skipping roads and transit",
            pbf.display()
        );
    }

    let scenario = scenario::to_scenario(&zones, &flows, roads.as_ref());
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

fn rail_summary(t: &Transit) -> Vec<String> {
    let dash = |o: &Option<String>| o.clone().unwrap_or_else(|| "-".into());
    let mut v: Vec<String> = t
        .routes
        .iter()
        .filter(|r| !matches!(r.mode.as_str(), "bus" | "trolleybus"))
        .map(|r| {
            format!(
                "{} | {} | {} | {} | {} stops",
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
