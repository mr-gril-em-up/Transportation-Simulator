//! End-to-end build on a tiny synthetic Travis County sample, written in the
//! real file formats (CenPop2020 block-group CSV, LODES8 gzipped CSV, OSM PBF).

mod support {
    pub mod pbf;
}

use data_pipeline::{aggregate::Level, build, census::Area, model::*, Config};

use flate2::{write::GzEncoder, Compression};
use std::io::Write;
use std::path::{Path, PathBuf};
use support::pbf::{Map, Member};

fn gz(path: &Path, text: &str) {
    let mut e = GzEncoder::new(std::fs::File::create(path).unwrap(), Compression::fast());
    e.write_all(text.as_bytes()).unwrap();
    e.finish().unwrap();
}

/// Minimal dBASE III table with GEOID (C12) and ALAND (N14), the two
/// fields the pipeline reads from TIGER/Line `tl_2020_48_bg.dbf`.
fn write_dbf(path: &Path, rows: &[(&str, u64)]) {
    let fields: [(&str, u8, u8); 2] = [("GEOID", b'C', 12), ("ALAND", b'N', 14)];
    let header_len = 32 + 32 * fields.len() + 1;
    let rec_len = 1 + fields.iter().map(|f| f.2 as usize).sum::<usize>();
    let mut b = vec![0u8; 32];
    b[0] = 3;
    b[4..8].copy_from_slice(&(rows.len() as u32).to_le_bytes());
    b[8..10].copy_from_slice(&(header_len as u16).to_le_bytes());
    b[10..12].copy_from_slice(&(rec_len as u16).to_le_bytes());
    for (name, ty, len) in fields {
        let mut d = [0u8; 32];
        d[..name.len()].copy_from_slice(name.as_bytes());
        d[11] = ty;
        d[16] = len;
        b.extend_from_slice(&d);
    }
    b.push(0x0D);
    for (geoid, aland) in rows {
        b.push(b' ');
        b.extend_from_slice(format!("{geoid:<12}{aland:>14}").as_bytes());
    }
    b.push(0x1A);
    std::fs::write(path, b).unwrap();
}

/// Four Travis block groups downtown and one in Williamson County (48491).
fn write_sample(raw: &Path) {
    std::fs::create_dir_all(raw).unwrap();
    std::fs::write(
        raw.join("CenPop2020_Mean_BG48.txt"),
        "\u{feff}STATEFP,COUNTYFP,TRACTCE,BLKGRPCE,POPULATION,LATITUDE,LONGITUDE\n\
         48,453,001100,1,1500,+30.270000,-097.745000\n\
         48,453,001100,2,1200,+30.275000,-097.740000\n\
         48,453,000603,1,2100,+30.300000,-097.720000\n\
         48,453,001804,1,1800,+30.350000,-097.700000\n\
         48,491,020301,1,2500,+30.510000,-097.680000\n",
    )
    .unwrap();
    // Block codes are block group + 3-digit block.
    gz(
        &raw.join("tx_rac_S000_JT00_2022.csv.gz"),
        "h_geocode,C000,CA01\n484530011001001,300,1\n484530011001002,200,1\n484530006031000,900,1\n484910203011000,1000,1\n",
    );
    gz(
        &raw.join("tx_wac_S000_JT00_2022.csv.gz"),
        "w_geocode,C000,CA01\n484530011001001,5000,1\n484530011002000,700,1\n484530018041000,50,1\n484910203011000,400,1\n",
    );
    gz(
        &raw.join("tx_od_main_JT00_2022.csv.gz"),
        "w_geocode,h_geocode,S000,SA01,createdate\n\
         484530011001001,484530006031000,400,0,20240101\n\
         484530011001002,484530006031001,100,0,20240101\n\
         484530011002000,484530018041000,80,0,20240101\n\
         484530011001001,484910203011000,600,0,20240101\n\
         484910203011000,484530011001001,30,0,20240101\n\
         481130001001000,484910203011000,999,0,20240101\n",
    );

    // ACS 5-year summary-file tables (Texas block-group rows): 011002 is suppressed and 018041 is
    // missing, so both exercise the tract/county fallback.
    std::fs::write(
        raw.join("acs5_bg48_2022_b19013.dat"),
        "GEO_ID|B19013_E001|B19013_M001\n\
         1500000US484530011001|90000|5000\n\
         1500000US484530011002|-666666666|-222222222\n\
         1500000US484530006031|50000|4000\n\
         1500000US484910203011|80000|6000\n",
    )
    .unwrap();
    std::fs::write(
        raw.join("acs5_bg48_2022_b25044.dat"),
        "GEO_ID|B25044_E001|B25044_E003|B25044_E010\n\
         1500000US484530011001|600|10|50\n\
         1500000US484530011002|0|0|0\n\
         1500000US484530006031|800|40|120\n\
         1500000US484910203011|900|5|4\n",
    )
    .unwrap();
    write_dbf(
        &raw.join("tl_2020_48_bg.dbf"),
        &[
            ("484530011001", 250_000),
            ("484530011002", 300_000),
            ("484530006031", 1_200_000),
        ],
    );

    let mut m = Map::default();
    let road = |m: &mut Map, id, lon, lat| m.node(id, lon, lat, &[]);
    road(&mut m, 1, -97.745, 30.265);
    road(&mut m, 2, -97.743, 30.270);
    road(&mut m, 3, -97.741, 30.275);
    road(&mut m, 4, -97.750, 30.270);
    road(&mut m, 5, -97.735, 30.270);
    road(&mut m, 6, -97.730, 30.290);
    road(&mut m, 9, -96.800, 32.780); // Dallas, outside the clip box
                                      // A disconnected fragment right next to the downtown centroid.
    road(&mut m, 40, -97.7449, 30.2701);
    road(&mut m, 41, -97.7448, 30.2705);
    m.node(
        20,
        -97.7431,
        30.2701,
        &[
            ("name", "Congress/5th"),
            ("public_transport", "stop_position"),
        ],
    );
    m.node(21, -97.7411, 30.2749, &[("name", "Capitol Station")]);
    m.node(30, -96.801, 32.781, &[("name", "Dallas Union Station")]);
    m.way(
        100,
        &[1, 2, 3],
        &[
            ("highway", "primary"),
            ("name", "Congress Avenue"),
            ("maxspeed", "35 mph"),
        ],
    );
    m.way(
        101,
        &[4, 2, 5],
        &[("highway", "residential"), ("name", "5th Street")],
    );
    m.way(102, &[3, 6], &[("highway", "motorway"), ("lanes", "3")]);
    m.way(103, &[5, 9], &[("highway", "residential")]);
    m.way(104, &[4, 6], &[("highway", "footway")]);
    m.way(105, &[40, 41], &[("highway", "residential")]);
    m.relation(
        200,
        vec![
            Member::Node(20, "stop"),
            Member::Way(100, ""),
            Member::Node(21, "stop_exit_only"),
        ],
        &[
            ("type", "route"),
            ("route", "bus"),
            ("ref", "801"),
            ("name", "MetroRapid 801"),
            ("operator", "CapMetro"),
        ],
    );
    m.relation(
        201,
        vec![Member::Node(30, "stop")],
        &[("type", "route"), ("route", "train")],
    );
    std::fs::write(raw.join("texas-latest.osm.pbf"), m.to_pbf()).unwrap();
}

fn read<T: serde::de::DeserializeOwned>(dir: &Path, name: &str) -> T {
    serde_json::from_reader(std::fs::File::open(dir.join(name)).unwrap()).unwrap()
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tsim-data-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn builds_travis_sample() {
    let root = tmp("sample");
    let (raw, out) = (root.join("raw"), root.join("out"));
    write_sample(&raw);
    let cfg = Config {
        raw,
        out: out.clone(),
        area: Area {
            prefixes: vec!["48453".into()],
        },
        year: 2022,
        margin_deg: 0.05,
        level: Level::BlockGroup,
        existing_lines: false,
    };
    let m = build(&cfg).unwrap();

    // Zones: Travis only, sorted by GEOID.
    let zones: Zones = read(&out, "zones.json");
    let ids: Vec<&str> = zones.zones.iter().map(|z| z.geoid.as_str()).collect();
    assert_eq!(
        ids,
        [
            "484530006031",
            "484530011001",
            "484530011002",
            "484530018041"
        ]
    );
    let z = |g: &str| zones.zones.iter().find(|z| z.geoid == g).unwrap();
    assert_eq!(z("484530011001").population, 1500);
    assert_eq!(
        z("484530011001").resident_workers,
        500,
        "blocks sum into their block group"
    );
    assert_eq!(z("484530011001").jobs, 5000);
    assert_eq!(
        z("484530011001").commute_in,
        600,
        "Williamson residents working downtown"
    );
    assert_eq!(z("484530011001").commute_out, 30);

    // ACS and TIGER: own values where published, tract then county fallback otherwise.
    let close = |a: Option<f64>, b: f64| (a.unwrap() - b).abs() < 1e-6;
    assert!(close(z("484530011001").median_income, 90_000.0));
    assert!(close(z("484530011001").zero_car_share, 0.1));
    assert!(
        close(z("484530011002").median_income, 90_000.0),
        "tract fallback"
    );
    assert!(
        close(z("484530011002").zero_car_share, 0.1),
        "tract fallback"
    );
    // County 48453 household-weighted: (90000*600 + 50000*800) / 1400; (60 + 160) / 1400.
    assert!(
        close(z("484530018041").median_income, 94_000_000.0 / 1400.0),
        "county fallback"
    );
    assert!(
        close(z("484530018041").zero_car_share, 220.0 / 1400.0),
        "county fallback"
    );
    assert!(close(z("484530006031").area_km2, 1.2));
    assert_eq!(z("484530018041").area_km2, None);
    assert_eq!(
        (m.acs_income_zones, m.acs_zero_car_zones, m.area_zones),
        (Some(2), Some(2), Some(3))
    );

    // Flows: two blocks in 006031 -> 011001 merge into one block-group flow.
    // scenario.json is sim-core's ScenarioInput; it must load and validate.
    let text = std::fs::read_to_string(out.join("scenario.json")).unwrap();
    let sc = sim_core::ScenarioInput::from_json(&text).unwrap();
    sc.validate().unwrap();
    assert!(
        (sc.zones[1].median_income - 90_000.0).abs() < 1e-6,
        "ACS reaches scenario.json"
    );
    assert_eq!(sc.zones[0].area_km2, Some(1.2));
    let named: Vec<(&str, &str, u32)> = sc
        .commutes
        .iter()
        .map(|f| {
            (
                zones.zones[f.home as usize].geoid.as_str(),
                zones.zones[f.work as usize].geoid.as_str(),
                f.workers as u32,
            )
        })
        .collect();
    assert_eq!(
        named,
        [
            ("484530006031", "484530011001", 500),
            ("484530018041", "484530011002", 80)
        ]
    );
    assert_eq!(m.internal_commuters, 580);
    assert_eq!(m.top_flows[0].workers, 500);

    // Roads: ways split at the shared node 2; the Dallas link and footway are dropped.
    let roads: Roads = read(&out, "roads.json");
    assert_eq!(roads.nodes.len(), 8, "6 connected + 2 in the fragment");
    assert_eq!(roads.edges.len(), 6);
    let congress: Vec<&RoadEdge> = roads
        .edges
        .iter()
        .filter(|e| e.name.as_deref() == Some("Congress Avenue"))
        .collect();
    assert_eq!(congress.len(), 2);
    assert!((congress[0].speed_kph - 56.3).abs() < 0.1, "35 mph");
    assert!(!congress[0].oneway);
    let fwy = roads.edges.iter().find(|e| e.class == "motorway").unwrap();
    assert!(fwy.oneway && fwy.lanes == 3);
    assert!(
        fwy.length_m > 1800.0 && fwy.length_m < 2200.0,
        "{}",
        fwy.length_m
    );

    // Transit: the bus route survives with ordered named stops; the Dallas train does not.
    let transit: Transit = read(&out, "transit.json");
    assert_eq!(transit.routes.len(), 1);
    assert_eq!(
        m.rail_routes.as_deref(),
        Some(&[][..]),
        "the only kept route is a bus"
    );
    let r = &transit.routes[0];
    assert_eq!((r.mode.as_str(), r.r#ref.as_deref()), ("bus", Some("801")));
    let stops: Vec<&str> = r.stops.iter().map(|s| s.name.as_deref().unwrap()).collect();
    assert_eq!(stops, ["Congress/5th", "Capitol Station"]);

    // Scenario roads: two-way 4-lane-default primary -> 2 per direction;
    // zones load onto local nodes, never the freeway-only node 6.
    // Simplified for routing: dead ends with no zone (Congress south end, the two
    // residential stubs, the freeway) are pruned and the fragment is dropped, so only
    // the two zone connectors and the Congress link between them remain. Ids are
    // roads.json ids.
    let ids: Vec<u32> = sc.road_nodes.iter().map(|n| n.id).collect();
    assert_eq!(ids, [1, 2]);
    assert_eq!(sc.road_links.len(), 1);
    assert_eq!((m.scenario_road_nodes, m.scenario_road_links), (2, 1));
    let fwy_end = roads
        .nodes
        .iter()
        .position(|n| near(*n, [-97.730, 30.290]))
        .unwrap() as u32;
    assert!(sc.zones.iter().all(|z| z.road_node != fwy_end));
    let downtown = &sc.zones[1]; // 484530011001 at (-97.745, 30.270)
    let n = roads.nodes[downtown.road_node as usize];
    assert!(
        near(n, [-97.743, 30.270]),
        "nearest local node in the main component, not the closer fragment"
    );
    assert!(sc.road_links.iter().any(|l| l.lanes == 2 && !l.oneway));
    assert!(
        sc.lines.is_empty(),
        "no GTFS in the sample, so no existing lines"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn whole_state_area_keeps_both_counties() {
    let root = tmp("state");
    let (raw, out) = (root.join("raw"), root.join("out"));
    write_sample(&raw);
    std::fs::remove_file(raw.join("texas-latest.osm.pbf")).unwrap();
    let cfg = Config {
        raw,
        out,
        area: Area {
            prefixes: vec!["48".into()],
        },
        year: 2022,
        margin_deg: 0.05,
        level: Level::BlockGroup,
        existing_lines: false,
    };
    let m = build(&cfg).unwrap();
    assert_eq!(m.zones, 5);
    // 500 + 80 + 600 + 30 internal now; Dallas County (48113) work block has no zone in the sample.
    assert_eq!(m.internal_commuters, 1210);
    assert_eq!(m.commute_out, 999);
    assert!(m.road_edges.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn county_level_merges_block_groups() {
    let root = tmp("county");
    let (raw, out) = (root.join("raw"), root.join("out"));
    write_sample(&raw);
    std::fs::remove_file(raw.join("texas-latest.osm.pbf")).unwrap();
    let cfg = Config {
        raw,
        out: out.clone(),
        area: Area {
            prefixes: vec!["48".into()],
        },
        year: 2022,
        margin_deg: 0.05,
        level: Level::County,
        existing_lines: false,
    };
    let m = build(&cfg).unwrap();
    assert_eq!((m.zones, m.block_groups, m.zone_level), (2, 5, "county"));
    // Totals do not change with the level.
    assert_eq!((m.population, m.internal_commuters), (9100, 1210));
    let zones: Zones = read(&out, "zones.json");
    let geoids: Vec<&str> = zones.zones.iter().map(|z| z.geoid.as_str()).collect();
    assert_eq!(geoids, ["48453", "48491"]);
    assert_eq!(zones.zones[0].population, 6600);
    let s: sim_core::ScenarioInput = read(&out, "scenario.json");
    assert_eq!(s.zones.len(), 2);
    let _ = std::fs::remove_dir_all(root);
}
