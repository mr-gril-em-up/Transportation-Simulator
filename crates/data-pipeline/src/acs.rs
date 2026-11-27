//! Zone attributes from the American Community Survey and TIGER/Line.
//!
//! - ACS 5-year detailed tables at block-group level, fetched from the Census
//!   API as JSON: median household income (B19013) and households with no
//!   vehicle (B25044, tenure by vehicles available).
//! - TIGER/Line 2020 block-group `ALAND` (land area, m²), read from the
//!   shapefile's `.dbf` attribute table. Polygons are not read yet.
//!
//! Both files are optional: without them zones keep sim-core's defaults.

use crate::model::Zone;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;

/// Census API variables requested by `scripts/fetch.sh`, in order.
pub const ACS_VARS: [&str; 4] = ["B19013_001E", "B25044_001E", "B25044_003E", "B25044_010E"];

/// ACS values for one block group. `None` where the Census suppresses the
/// estimate (negative sentinel such as -666666666) or the base is zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcsRow {
    pub median_income: Option<f64>,
    pub zero_car_share: Option<f64>,
    pub households: f64,
}

/// Parses the Census API JSON (an array of rows, the first being headers)
/// into a map keyed by 12-digit block group GEOID.
pub fn read_acs(json: &str) -> Result<HashMap<String, AcsRow>> {
    let rows: Vec<Vec<Option<String>>> = serde_json::from_str(json).context("ACS JSON")?;
    let Some((head, body)) = rows.split_first() else {
        bail!("ACS file is empty");
    };
    let h = |name: &str| {
        head.iter()
            .position(|c| c.as_deref() == Some(name))
            .with_context(|| format!("ACS file lacks column {name}"))
    };
    let (inc, hh, own0, rent0) = (
        h(ACS_VARS[0])?,
        h(ACS_VARS[1])?,
        h(ACS_VARS[2])?,
        h(ACS_VARS[3])?,
    );
    let (st, co, tr, bg) = (h("state")?, h("county")?, h("tract")?, h("block group")?);
    let num = |r: &[Option<String>], i: usize| -> Option<f64> {
        r[i].as_deref()?.parse::<f64>().ok().filter(|v| *v >= 0.0)
    };
    let mut out = HashMap::new();
    for r in body {
        let s = |i: usize| r[i].as_deref().unwrap_or("");
        let geoid = format!("{}{}{}{}", s(st), s(co), s(tr), s(bg));
        let households = num(r, hh).unwrap_or(0.0);
        let zero = match (num(r, own0), num(r, rent0)) {
            (Some(a), Some(b)) if households > 0.0 => Some((a + b) / households),
            _ => None,
        };
        out.insert(
            geoid,
            AcsRow {
                median_income: num(r, inc).filter(|v| *v > 0.0),
                zero_car_share: zero,
                households,
            },
        );
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
        let j = r#"[["B19013_001E","B25044_001E","B25044_003E","B25044_010E","state","county","tract","block group"],
            ["71250","500","10","40","48","453","001101","1"],
            ["-666666666","0","0","0","48","453","001101","2"],
            [null,"200","5","15","48","453","001102","1"]]"#;
        let m = read_acs(j).unwrap();
        let a = m["484530011011"];
        assert_eq!(a.median_income, Some(71250.0));
        assert!((a.zero_car_share.unwrap() - 0.1).abs() < 1e-12);
        let b = m["484530011012"];
        assert_eq!((b.median_income, b.zero_car_share), (None, None));
        assert_eq!(m["484530011021"].median_income, None);
        assert!((m["484530011021"].zero_car_share.unwrap() - 0.1).abs() < 1e-12);
    }
}
