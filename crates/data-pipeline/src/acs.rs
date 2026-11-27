//! Zone attributes from the American Community Survey and TIGER/Line.
//!
//! - ACS 5-year detailed tables at block-group level, from the Census
//!   Bureau's table-based summary files (pipe-delimited, one file per table,
//!   no API key needed): median household income (B19013) and households
//!   with no vehicle (B25044, tenure by vehicles available). `fetch.sh` keeps
//!   only Texas block-group rows. Means of transportation to work (B08301)
//!   is summed over the area into commute mode shares for calibration.
//! - TIGER/Line 2020 block-group `ALAND` (land area, m²), read from the
//!   shapefile's `.dbf` attribute table. Polygons are not read yet.
//!
//! Both are optional: without them zones keep sim-core's defaults.

use crate::model::Zone;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::HashMap;

/// ACS values for one block group. `None` where the Census suppresses the
/// estimate (blank, or a negative sentinel such as -666666666) or the base
/// is zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcsRow {
    pub median_income: Option<f64>,
    pub zero_car_share: Option<f64>,
    pub households: f64,
}

/// Block group GEOID from a summary-file `GEO_ID` such as
/// `1500000US484530011001`; `None` for other summary levels.
fn bg_geoid(geo_id: &str) -> Option<&str> {
    let g = geo_id.strip_prefix("1500000US")?;
    (g.len() == 12).then_some(g)
}

/// Area-wide commute mode shares from ACS B08301 (workers 16 and over),
/// summed over the study area's block groups. Shares are of all workers,
/// including those who work from home, and add up to 1.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CommuteShares {
    /// Block groups with a B08301 row.
    pub block_groups: usize,
    /// Workers 16 and over (`B08301_E001`), summed.
    pub workers: f64,
    pub drove_alone: f64,
    pub carpool: f64,
    /// Public transportation excluding taxicab (`_E010`).
    pub transit: f64,
    pub walk: f64,
    pub bike: f64,
    /// Worked from home (`_E021`).
    pub wfh: f64,
    /// Taxicab, motorcycle and other means (`_E016`, `_E017`, `_E020`).
    pub taxi_motorcycle_other: f64,
}

/// Sums B08301 over `geoids` (12-digit block groups) into mode shares.
/// Block-group estimates are summed, which equals the published area total
/// up to rounding. `None` if no listed block group has workers.
pub fn commute_shares<'a>(
    b08301: &str,
    geoids: impl Iterator<Item = &'a str>,
) -> Result<Option<CommuteShares>> {
    const COLS: [&str; 10] = [
        "B08301_E001",
        "B08301_E003",
        "B08301_E004",
        "B08301_E010",
        "B08301_E019",
        "B08301_E018",
        "B08301_E021",
        "B08301_E016",
        "B08301_E017",
        "B08301_E020",
    ];
    let t = read_table(b08301, &COLS)?;
    let mut sum = [0.0; 10];
    let mut n = 0;
    for g in geoids {
        if let Some(v) = t.get(g) {
            n += 1;
            for (s, x) in sum.iter_mut().zip(v) {
                *s += x.unwrap_or(0.0);
            }
        }
    }
    let w = sum[0];
    if w <= 0.0 {
        return Ok(None);
    }
    let r = |x: f64| (x / w * 10_000.0).round() / 10_000.0;
    Ok(Some(CommuteShares {
        block_groups: n,
        workers: w,
        drove_alone: r(sum[1]),
        carpool: r(sum[2]),
        transit: r(sum[3]),
        walk: r(sum[4]),
        bike: r(sum[5]),
        wfh: r(sum[6]),
        taxi_motorcycle_other: r(sum[7] + sum[8] + sum[9]),
    }))
}

/// Reads one pipe-delimited table: header row, then one row per geography.
/// Returns the requested columns by block group GEOID.
fn read_table(text: &str, cols: &[&str]) -> Result<HashMap<String, Vec<Option<f64>>>> {
    let mut lines = text.lines();
    let head: Vec<&str> = lines
        .next()
        .context("ACS table is empty")?
        .split('|')
        .collect();
    let pos = |name: &str| {
        head.iter()
            .position(|c| c.trim().eq_ignore_ascii_case(name))
            .with_context(|| format!("ACS table lacks column {name}"))
    };
    let geo = pos("GEO_ID")?;
    let idx: Vec<usize> = cols.iter().map(|c| pos(c)).collect::<Result<_>>()?;
    let mut out = HashMap::new();
    for line in lines {
        let f: Vec<&str> = line.split('|').collect();
        let Some(g) = f.get(geo).and_then(|g| bg_geoid(g.trim())) else {
            continue;
        };
        let vals = idx
            .iter()
            .map(|&i| f.get(i)?.trim().parse::<f64>().ok().filter(|v| *v >= 0.0))
            .collect();
        out.insert(g.to_string(), vals);
    }
    Ok(out)
}

/// Joins the B19013 and B25044 summary-file tables into one map keyed by
/// 12-digit block group GEOID.
pub fn read_acs(b19013: &str, b25044: &str) -> Result<HashMap<String, AcsRow>> {
    let inc = read_table(b19013, &["B19013_E001"])?;
    let veh = read_table(b25044, &["B25044_E001", "B25044_E003", "B25044_E010"])?;
    let mut out = HashMap::new();
    for (g, v) in &veh {
        let households = v[0].unwrap_or(0.0);
        let zero = match (v[1], v[2]) {
            (Some(a), Some(b)) if households > 0.0 => Some((a + b) / households),
            _ => None,
        };
        out.insert(
            g.clone(),
            AcsRow {
                median_income: inc.get(g).and_then(|r| r[0]).filter(|v| *v > 0.0),
                zero_car_share: zero,
                households,
            },
        );
    }
    for (g, r) in &inc {
        out.entry(g.clone()).or_insert(AcsRow {
            median_income: r[0].filter(|v| *v > 0.0),
            zero_car_share: None,
            households: 0.0,
        });
    }
    Ok(out)
}

/// Reads `GEOID` and `ALAND` from a dBASE III table (the `.dbf` of a
/// TIGER/Line shapefile). Returns land area in m² by GEOID.
pub fn read_dbf_aland(bytes: &[u8]) -> Result<HashMap<String, f64>> {
    anyhow::ensure!(bytes.len() >= 32, "dbf too short");
    let n = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
    let header_len = u16::from_le_bytes(bytes[8..10].try_into()?) as usize;
    let rec_len = u16::from_le_bytes(bytes[10..12].try_into()?) as usize;
    // Field descriptors: 32 bytes each from offset 32 until 0x0D.
    let (mut fields, mut off, mut pos) = (HashMap::new(), 1usize, 32usize);
    while pos + 32 <= header_len && bytes[pos] != 0x0D {
        let d = &bytes[pos..pos + 32];
        let name: String = d[..11]
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect();
        let len = d[16] as usize;
        fields.insert(name, (off, len));
        off += len;
        pos += 32;
    }
    let get = |name: &str| {
        fields
            .get(name)
            .copied()
            .with_context(|| format!("dbf lacks {name}"))
    };
    let (g, a) = (get("GEOID")?, get("ALAND")?);
    let mut out = HashMap::with_capacity(n);
    for i in 0..n {
        let start = header_len + i * rec_len;
        let Some(rec) = bytes.get(start..start + rec_len) else {
            bail!("dbf truncated at record {i}");
        };
        if rec[0] == b'*' {
            continue; // deleted
        }
        let field =
            |(o, l): (usize, usize)| String::from_utf8_lossy(&rec[o..o + l]).trim().to_string();
        let aland: f64 = field(a)
            .parse()
            .with_context(|| format!("ALAND in record {i}"))?;
        out.insert(field(g), aland);
    }
    Ok(out)
}

/// Fills zone ACS fields and area. A block group whose estimate is suppressed
/// takes the household-weighted mean of the other block groups in its tract,
/// then its county; only if those are empty too does it keep the default.
/// Returns (zones with own income, own zero-car share, area).
pub fn apply(
    zones: &mut [Zone],
    acs: Option<&HashMap<String, AcsRow>>,
    aland: Option<&HashMap<String, f64>>,
) -> (usize, usize, usize) {
    let mut counts = (0, 0, 0);
    if let Some(aland) = aland {
        for z in zones.iter_mut() {
            z.area_km2 = aland.get(&z.geoid).map(|m2| m2 / 1e6).filter(|a| *a > 0.0);
            counts.2 += z.area_km2.is_some() as usize;
        }
    }
    let Some(acs) = acs else {
        return counts;
    };
    // Household-weighted sums by tract (11 digits) and county (5 digits).
    let mut groups: HashMap<&str, [f64; 4]> = HashMap::new();
    for (geoid, r) in acs {
        for key in [&geoid[..11.min(geoid.len())], &geoid[..5.min(geoid.len())]] {
            let g = groups.entry(key).or_default();
            if let Some(v) = r.median_income {
                g[0] += v * r.households;
                g[1] += r.households;
            }
            if let Some(v) = r.zero_car_share {
                g[2] += v * r.households;
                g[3] += r.households;
            }
        }
    }
    let fallback = |geoid: &str, k: usize| {
        [&geoid[..11], &geoid[..5]].iter().find_map(|key| {
            let g = groups.get(key)?;
            (g[k + 1] > 0.0).then(|| g[k] / g[k + 1])
        })
    };
    for z in zones.iter_mut() {
        let r = acs.get(&z.geoid);
        z.median_income = r.and_then(|r| r.median_income);
        z.zero_car_share = r.and_then(|r| r.zero_car_share);
        counts.0 += z.median_income.is_some() as usize;
        counts.1 += z.zero_car_share.is_some() as usize;
        z.median_income = z.median_income.or_else(|| fallback(&z.geoid, 0));
        z.zero_car_share = z.zero_car_share.or_else(|| fallback(&z.geoid, 2));
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acs_parses_and_flags_suppressed() {
        let inc = "GEO_ID|B19013_E001|B19013_M001\n\
                   0400000US48|76292|300\n\
                   1500000US484530011011|71250|9000\n\
                   1500000US484530011012|-666666666|-222222222\n\
                   1500000US484530011021||\n";
        let veh = "GEO_ID|B25044_E001|B25044_E002|B25044_E003|B25044_E010\n\
                   1500000US484530011011|500|300|10|40\n\
                   1500000US484530011012|0|0|0|0\n\
                   1500000US484530011021|200|100|5|15\n";
        let m = read_acs(inc, veh).unwrap();
        assert_eq!(m.len(), 3, "state row skipped");
        let a = m["484530011011"];
        assert_eq!(a.median_income, Some(71250.0));
        assert!((a.zero_car_share.unwrap() - 0.1).abs() < 1e-12);
        let b = m["484530011012"];
        assert_eq!((b.median_income, b.zero_car_share), (None, None));
        assert_eq!(m["484530011021"].median_income, None);
        assert!((m["484530011021"].zero_car_share.unwrap() - 0.1).abs() < 1e-12);
    }

    #[test]
    fn commute_shares_sum_listed_block_groups() {
        let head = "GEO_ID|B08301_E001|B08301_E003|B08301_E004|B08301_E010|B08301_E016|B08301_E017|B08301_E018|B08301_E019|B08301_E020|B08301_E021";
        let text = format!(
            "{head}\n0500000US48453|999|0|0|0|0|0|0|0|0|0\n\
             1500000US484530011001|100|60|10|5|1|1|2|3|2|16\n\
             1500000US484530011002|100|80|5|5|0|0|0|5|0|5\n\
             1500000US484910203011|500|500|0|0|0|0|0|0|0|0\n"
        );
        let c = commute_shares(
            &text,
            ["484530011001", "484530011002", "484530099991"].into_iter(),
        )
        .unwrap()
        .unwrap();
        assert_eq!((c.block_groups, c.workers), (2, 200.0));
        assert_eq!((c.drove_alone, c.carpool, c.transit), (0.7, 0.075, 0.05));
        assert_eq!(
            (c.walk, c.bike, c.wfh, c.taxi_motorcycle_other),
            (0.04, 0.01, 0.105, 0.02)
        );
    }
}
