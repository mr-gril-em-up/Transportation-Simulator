//! Zones and commute demand from Census Bureau files.
//!
//! - Zones: 2020 Centers of Population by block group (CenPop2020_Mean_BG48.txt),
//!   which gives each block group's population and population-weighted centroid.
//! - Demand: LODES8 OD / RAC / WAC files (2020 census blocks), aggregated from
//!   15-digit block codes to 12-digit block groups.

use crate::model::{Flow, Zone};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::Read;

/// Which block groups belong to the study area: GEOID prefixes such as
/// "48453" (Travis County) or "48" (all of Texas).
#[derive(Debug, Clone)]
pub struct Area {
    pub prefixes: Vec<String>,
}

impl Area {
    pub fn contains(&self, geoid: &str) -> bool {
        self.prefixes.iter().any(|p| geoid.starts_with(p.as_str()))
    }
}

fn csv_reader<R: Read>(r: R) -> csv::Reader<R> {
    csv::ReaderBuilder::new().flexible(true).from_reader(r)
}

/// Column index by name, ignoring case and a UTF-8 BOM (the CenPop files carry one).
fn col(headers: &csv::StringRecord, name: &str) -> Result<usize> {
    headers
        .iter()
        .position(|h| h.trim_start_matches('\u{feff}').eq_ignore_ascii_case(name))
        .with_context(|| format!("missing column {name}"))
}

/// Reads block-group centers of population, keeping block groups in `area`.
/// Output is sorted by GEOID so zone indexes are stable across runs.
/// The published layout is STATEFP,COUNTYFP,TRACTCE,BLKGRPCE,POPULATION,LATITUDE,LONGITUDE;
/// a header row is added if the file lacks one.
pub fn read_zones<R: Read>(mut r: R, area: &Area) -> Result<Vec<Zone>> {
    let mut text = String::new();
    r.read_to_string(&mut text)?;
    if !text
        .lines()
        .next()
        .unwrap_or("")
        .to_ascii_uppercase()
        .contains("STATEFP")
    {
        text.insert_str(
            0,
            "STATEFP,COUNTYFP,TRACTCE,BLKGRPCE,POPULATION,LATITUDE,LONGITUDE\n",
        );
    }
    let mut rdr = csv_reader(text.as_bytes());
    let h = rdr.headers()?.clone();
    let (st, co, tr, bg) = (
        col(&h, "STATEFP")?,
        col(&h, "COUNTYFP")?,
        col(&h, "TRACTCE")?,
        col(&h, "BLKGRPCE")?,
    );
    let (pop, lat, lon) = (
        col(&h, "POPULATION")?,
        col(&h, "LATITUDE")?,
        col(&h, "LONGITUDE")?,
    );
    let mut zones = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        let geoid = format!("{}{}{}{}", &rec[st], &rec[co], &rec[tr], &rec[bg]);
        if geoid.len() != 12 {
            bail!("bad block group id {geoid}");
        }
        if !area.contains(&geoid) {
            continue;
        }
        zones.push(Zone {
            geoid,
            lon: rec[lon].trim().parse()?,
            lat: rec[lat].trim().parse()?,
            population: rec[pop].trim().parse()?,
            resident_workers: 0,
            jobs: 0,
            commute_out: 0,
            commute_in: 0,
        });
    }
    zones.sort_by(|a, b| a.geoid.cmp(&b.geoid));
    Ok(zones)
}

pub fn index_by_geoid(zones: &[Zone]) -> HashMap<String, u32> {
    zones
        .iter()
        .enumerate()
        .map(|(i, z)| (z.geoid.clone(), i as u32))
        .collect()
}

/// Sums a LODES RAC or WAC file's C000 (total jobs) into zones.
/// `geocode_col` is "h_geocode" for RAC, "w_geocode" for WAC.
/// Returns the total for blocks inside the area, for the manifest's checks.
pub fn add_lodes_totals<R: Read>(
    r: R,
    geocode_col: &str,
    zones: &mut [Zone],
    idx: &HashMap<String, u32>,
    is_rac: bool,
) -> Result<u64> {
    let mut rdr = csv_reader(r);
    let h = rdr.headers()?.clone();
    let (g, c) = (col(&h, geocode_col)?, col(&h, "C000")?);
    let mut total = 0u64;
    for rec in rdr.records() {
        let rec = rec?;
        let Some(&i) = rec[g].get(..12).and_then(|bg| idx.get(bg)) else {
            continue;
        };
        let n: u32 = rec[c].trim().parse()?;
        let z = &mut zones[i as usize];
        if is_rac {
            z.resident_workers += n;
        } else {
            z.jobs += n;
        }
        total += n as u64;
    }
    Ok(total)
}

/// Aggregates a LODES OD file (S000 = all jobs) to block-group flows.
/// Flows with both ends in the area become `Flow`s (sorted by home, work);
/// flows with one end in the area add to that zone's commute_out / commute_in.
pub fn read_flows<R: Read>(
    r: R,
    zones: &mut [Zone],
    idx: &HashMap<String, u32>,
) -> Result<Vec<Flow>> {
    let mut rdr = csv_reader(r);
    let h = rdr.headers()?.clone();
    let (w, hh, s) = (
        col(&h, "w_geocode")?,
        col(&h, "h_geocode")?,
        col(&h, "S000")?,
    );
    let mut acc: HashMap<(u32, u32), u32> = HashMap::new();
    for rec in rdr.records() {
        let rec = rec?;
        let home = rec[hh].get(..12).and_then(|bg| idx.get(bg)).copied();
        let work = rec[w].get(..12).and_then(|bg| idx.get(bg)).copied();
        if home.is_none() && work.is_none() {
            continue;
        }
        let n: u32 = rec[s].trim().parse()?;
        match (home, work) {
            (Some(h), Some(w)) => *acc.entry((h, w)).or_default() += n,
            (Some(h), None) => zones[h as usize].commute_out += n,
            (None, Some(w)) => zones[w as usize].commute_in += n,
            (None, None) => unreachable!(),
        }
    }
    let mut flows: Vec<Flow> = acc
        .into_iter()
        .map(|((home, work), workers)| Flow {
            home,
            work,
            workers,
        })
        .collect();
    flows.sort_by_key(|f| (f.home, f.work));
    Ok(flows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_without_header_row() {
        let text =
            "48,453,001100,1,1500,+30.270000,-097.745000\n48,491,020301,1,2500,+30.51,-97.68\n";
        let area = Area {
            prefixes: vec!["48453".into()],
        };
        let z = read_zones(text.as_bytes(), &area).unwrap();
        assert_eq!(z.len(), 1);
        assert_eq!(
            (z[0].geoid.as_str(), z[0].population),
            ("484530011001", 1500)
        );
        assert_eq!(z[0].lon, -97.745);
    }
}
