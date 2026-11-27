use anyhow::{bail, Result};
use data_pipeline::{aggregate::Level, build, census::Area, Config};
use std::path::PathBuf;

const USAGE: &str = "usage: tsim-data [--raw DIR] [--out DIR] [--area 48453[,48491...]] [--year 2023] [--margin 0.05] [--level bg|tract|county] [--existing-lines]

Builds scenario.json (sim-core input), zones.json, roads.json, transit.json and manifest.json.
--area takes GEOID prefixes: 48453 = Travis County, 48 = all of Texas.
--level merges block groups into census tracts or counties (default bg).
--existing-lines puts today's transit network into scenario.json lines (always in existing_lines.json).";

fn main() -> Result<()> {
    let mut cfg = Config {
        raw: PathBuf::from("data/raw"),
        out: PathBuf::from("data/out/travis"),
        area: Area {
            prefixes: vec!["48453".into()],
        },
        year: 2023,
        margin_deg: 0.05,
        level: Level::BlockGroup,
        existing_lines: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut val = || {
            args.next()
                .ok_or_else(|| anyhow::anyhow!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--raw" => cfg.raw = val()?.into(),
            "--out" => cfg.out = val()?.into(),
            "--area" => cfg.area.prefixes = val()?.split(',').map(str::to_string).collect(),
            "--year" => cfg.year = val()?.parse()?,
            "--margin" => cfg.margin_deg = val()?.parse()?,
            "--level" => cfg.level = Level::parse(&val()?)?,
            "--existing-lines" => cfg.existing_lines = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => bail!("unknown argument {flag}\n{USAGE}"),
        }
    }
    let t = std::time::Instant::now();
    let m = build(&cfg)?;
    println!("{}", serde_json::to_string_pretty(&m)?);
    eprintln!(
        "built {} in {:.1}s",
        cfg.out.display(),
        t.elapsed().as_secs_f64()
    );
    Ok(())
}
