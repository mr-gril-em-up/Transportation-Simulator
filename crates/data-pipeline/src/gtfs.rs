//! Weekday headways for existing transit routes from a GTFS static feed
//! (CapMetro's, for Travis). OSM route relations give the stops but no
//! frequencies; GTFS gives the timetable. This module reduces the timetable
//! to a few numbers per route and attaches them to the OSM routes by `ref`.
//!
//! Method (DATA-6):
//! - Service day: the Wednesday with the most scheduled trips, from
//!   `calendar.txt` and `calendar_dates.txt` (either may be missing).
//! - Per route (`route_short_name`) and direction, each trip's first
//!   departure; each window takes the more frequent direction.
//! - Peak headway = 120 min / departures in 07:00–09:00; midday headway =
//!   300 min / departures in 10:00–15:00; span = last − first departure.
//!   A window with no departures gives `None`.

use crate::model::Transit;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Headways for one GTFS route on the chosen weekday.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RouteHeadway {
    pub peak_headway_min: Option<f64>,
    pub midday_headway_min: Option<f64>,
    pub span_hours: f64,
    /// Trips that day in the busier direction.
    pub trips: usize,
}

/// Days since 1970-01-01 for a GTFS `YYYYMMDD` date.
fn day(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() != 8 {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (
        s[..4].parse().ok()?,
        s[4..6].parse().ok()?,
        s[6..].parse().ok()?,
    );
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn is_wednesday(d: i64) -> bool {
    // 1970-01-01 was a Thursday.
    (d + 1).rem_euclid(7) == 0
}

/// `HH:MM:SS` (hours may exceed 24) to minutes after midnight.
fn minutes(s: &str) -> Option<f64> {
    let mut it = s.trim().split(':');
    let h: f64 = it.next()?.parse().ok()?;
    let m: f64 = it.next()?.parse().ok()?;
    let sec: f64 = it.next().unwrap_or("0").parse().ok()?;
    Some(h * 60.0 + m + sec / 60.0)
}

fn reader(dir: &Path, name: &str) -> Result<Option<csv::Reader<std::fs::File>>> {
    let p = dir.join(name);
    if !p.exists() {
        return Ok(None);
    }
    Ok(Some(
        csv::ReaderBuilder::new()
            .flexible(true)
            .trim(csv::Trim::All)
            .from_path(&p)
            .with_context(|| format!("open {}", p.display()))?,
    ))
}

/// Column index by name, tolerating a UTF-8 BOM on the first header.
fn col(h: &csv::StringRecord, name: &str) -> Result<usize> {
    h.iter()
        .position(|c| c.trim_start_matches('\u{feff}') == name)
        .with_context(|| format!("GTFS file lacks column {name}"))
}

/// Reads an unzipped GTFS feed and returns headways by `route_short_name`.
pub fn read_headways(dir: &Path) -> Result<HashMap<String, RouteHeadway>> {
    // Routes.
    let mut r = reader(dir, "routes.txt")?.context("routes.txt missing")?;
    let h = r.headers()?.clone();
    let (rid, rsn) = (col(&h, "route_id")?, col(&h, "route_short_name")?);
    let mut short: HashMap<String, String> = HashMap::new();
    for rec in r.records() {
        let rec = rec?;
        short.insert(rec[rid].to_string(), rec[rsn].to_string());
    }

    // Trips.
    let mut t = reader(dir, "trips.txt")?.context("trips.txt missing")?;
    let h = t.headers()?.clone();
    let (troute, tsvc, tid) = (
        col(&h, "route_id")?,
        col(&h, "service_id")?,
        col(&h, "trip_id")?,
    );
    let tdir = col(&h, "direction_id").ok();
    struct Trip {
        route: String,
        service: String,
        dir: String,
    }
    let mut trips: HashMap<String, Trip> = HashMap::new();
    for rec in t.records() {
        let rec = rec?;
        trips.insert(
            rec[tid].to_string(),
            Trip {
                route: rec[troute].to_string(),
                service: rec[tsvc].to_string(),
                dir: tdir.and_then(|i| rec.get(i)).unwrap_or("0").to_string(),
            },
        );
    }

    // Services active on each candidate Wednesday.
    let mut weekly: Vec<(String, i64, i64)> = Vec::new();
    if let Some(mut c) = reader(dir, "calendar.txt")? {
        let h = c.headers()?.clone();
        let (sid, wed, s, e) = (
            col(&h, "service_id")?,
            col(&h, "wednesday")?,
            col(&h, "start_date")?,
            col(&h, "end_date")?,
        );
        for rec in c.records() {
            let rec = rec?;
            if &rec[wed] == "1" {
                if let (Some(a), Some(b)) = (day(&rec[s]), day(&rec[e])) {
                    weekly.push((rec[sid].to_string(), a, b));
                }
            }
        }
    }
    let mut added: HashMap<i64, HashSet<String>> = HashMap::new();
    let mut removed: HashMap<i64, HashSet<String>> = HashMap::new();
    if let Some(mut c) = reader(dir, "calendar_dates.txt")? {
        let h = c.headers()?.clone();
        let (sid, d, ex) = (
            col(&h, "service_id")?,
            col(&h, "date")?,
            col(&h, "exception_type")?,
        );
        for rec in c.records() {
            let rec = rec?;
            let Some(dd) = day(&rec[d]) else { continue };
            let m = if &rec[ex] == "1" {
                &mut added
            } else {
                &mut removed
            };
            m.entry(dd).or_default().insert(rec[sid].to_string());
        }
    }
    let mut candidates: HashSet<i64> = added.keys().copied().filter(|&d| is_wednesday(d)).collect();
    for &(_, a, b) in &weekly {
        let first = a + (0..7).find(|k| is_wednesday(a + k)).unwrap_or(0);
        candidates.extend((first..=b.min(a + 400)).step_by(7));
    }
    let mut trips_per_service: HashMap<&str, usize> = HashMap::new();
    for tr in trips.values() {
        *trips_per_service.entry(tr.service.as_str()).or_default() += 1;
    }
    let active = |d: i64| -> HashSet<String> {
        let mut s: HashSet<String> = weekly
            .iter()
            .filter(|(_, a, b)| *a <= d && d <= *b)
            .map(|(id, _, _)| id.clone())
            .collect();
        if let Some(r) = removed.get(&d) {
            s.retain(|id| !r.contains(id));
        }
        if let Some(a) = added.get(&d) {
            s.extend(a.iter().cloned());
        }
        s
    };
    let mut days: Vec<i64> = candidates.into_iter().collect();
    days.sort();
    let best = days
        .iter()
        .map(|&d| {
            let n: usize = active(d)
                .iter()
                .map(|s| trips_per_service.get(s.as_str()).copied().unwrap_or(0))
                .sum();
            (n, std::cmp::Reverse(d), d)
        })
        .max()
        .context("GTFS feed has no Wednesday service")?
        .2;
    let services = active(best);

    // First departure of each trip running that day.
    let mut st = reader(dir, "stop_times.txt")?.context("stop_times.txt missing")?;
    let h = st.headers()?.clone();
    let (sttrip, stdep, stseq) = (
        col(&h, "trip_id")?,
        col(&h, "departure_time")?,
        col(&h, "stop_sequence")?,
    );
    let mut first: HashMap<String, (u32, f64)> = HashMap::new();
    for rec in st.records() {
        let rec = rec?;
        let Some(tr) = trips.get(&rec[sttrip]) else {
            continue;
        };
        if !services.contains(&tr.service) {
            continue;
        }
        let (Ok(seq), Some(dep)) = (rec[stseq].parse::<u32>(), minutes(&rec[stdep])) else {
            continue;
        };
        let e = first
            .entry(rec[sttrip].to_string())
            .or_insert((u32::MAX, dep));
        if seq < e.0 {
            *e = (seq, dep);
        }
    }

    // Group by route short name and direction.
    let mut deps: HashMap<(String, String), Vec<f64>> = HashMap::new();
    for (id, (_, dep)) in &first {
        let tr = &trips[id];
        let name = short
            .get(&tr.route)
            .cloned()
            .unwrap_or_else(|| tr.route.clone());
        deps.entry((name, tr.dir.clone())).or_default().push(*dep);
    }
    let mut out: HashMap<String, RouteHeadway> = HashMap::new();
    for ((name, _), mut d) in deps {
        d.sort_by(f64::total_cmp);
        let count = |a: f64, b: f64| d.iter().filter(|&&x| x >= a && x < b).count();
        let hw = |a: f64, b: f64| {
            let n = count(a, b);
            (n > 0).then(|| ((b - a) / n as f64 * 10.0).round() / 10.0)
        };
        let h = RouteHeadway {
            peak_headway_min: hw(420.0, 540.0),
            midday_headway_min: hw(600.0, 900.0),
            span_hours: ((d[d.len() - 1] - d[0]) / 60.0 * 10.0).round() / 10.0,
            trips: d.len(),
        };
        // Combine directions per window: a peak-direction express runs one
        // way in the morning and the other in the evening.
        let best = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (x, y) => x.or(y),
        };
        out.entry(name)
            .and_modify(|o| {
                o.peak_headway_min = best(o.peak_headway_min, h.peak_headway_min);
                o.midday_headway_min = best(o.midday_headway_min, h.midday_headway_min);
                o.span_hours = o.span_hours.max(h.span_hours);
                o.trips = o.trips.max(h.trips);
            })
            .or_insert(h);
    }
    Ok(out)
}

/// True if an OSM `operator` tag can be CapMetro: missing, or naming it.
/// Keeps Amtrak (Texas Eagle trains 21 and 22) and other operators from
/// matching CapMetro route numbers.
pub fn is_capmetro(operator: Option<&str>) -> bool {
    operator.is_none_or(|o| {
        let o: String = o.to_lowercase().split_whitespace().collect();
        o.contains("capmetro") || o.contains("capitalmetro")
    })
}

/// Copies headways onto CapMetro OSM routes whose `ref` equals a GTFS
/// `route_short_name`. Returns the number of OSM routes matched.
pub fn apply(transit: &mut Transit, hw: &HashMap<String, RouteHeadway>) -> usize {
    let mut n = 0;
    for r in &mut transit.routes {
        if !is_capmetro(r.operator.as_deref()) {
            continue;
        }
        if let Some(h) = r.r#ref.as_deref().and_then(|k| hw.get(k.trim())) {
            r.peak_headway_min = h.peak_headway_min;
            r.midday_headway_min = h.midday_headway_min;
            r.span_hours = Some(h.span_hours);
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_capmetro_operators_match() {
        assert!(is_capmetro(None));
        assert!(is_capmetro(Some(
            "Capital Metropolitan Transportation Authority"
        )));
        assert!(is_capmetro(Some("CapMetro")));
        assert!(is_capmetro(Some("Cap Metro")));
        assert!(!is_capmetro(Some("Amtrak")));
        assert!(!is_capmetro(Some("CARTS")));
    }

    #[test]
    fn dates_and_weekdays() {
        assert_eq!(day("19700101"), Some(0));
        assert!(is_wednesday(day("20261125").unwrap()));
        assert!(!is_wednesday(day("20261126").unwrap()));
        assert_eq!(minutes("25:30:00"), Some(1530.0));
    }

    #[test]
    fn headways_from_a_tiny_feed() {
        let dir = std::env::temp_dir().join(format!("tsim-gtfs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let w = |n: &str, s: &str| std::fs::write(dir.join(n), s).unwrap();
        w(
            "routes.txt",
            "\u{feff}route_id,route_short_name\nR7,7\nR550,550\n",
        );
        // WK runs Mon-Fri all November 2026; SAT runs weekends only.
        w(
            "calendar.txt",
            "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\n\
             WK,1,1,1,1,1,0,0,20261101,20261130\nSAT,0,0,0,0,0,1,1,20261101,20261130\n",
        );
        // Thanksgiving week: WK removed on Wed 25th does not matter, other Wednesdays win.
        w(
            "calendar_dates.txt",
            "service_id,date,exception_type\nWK,20261125,2\n",
        );
        let mut trips = String::from("route_id,service_id,trip_id,direction_id\n");
        let mut st = String::from("trip_id,arrival_time,departure_time,stop_id,stop_sequence\n");
        // Route 7 direction 0: every 15 min 07:00-09:00, every 30 min 10:00-15:00.
        let mut add = |route: &str, svc: &str, id: String, dir: u8, t: u32| {
            trips.push_str(&format!("{route},{svc},{id},{dir}\n"));
            let hm = |m: u32| format!("{:02}:{:02}:00", m / 60, m % 60);
            st.push_str(&format!("{id},{},{},A,1\n", hm(t), hm(t)));
            st.push_str(&format!("{id},{},{},B,2\n", hm(t + 20), hm(t + 20)));
        };
        for (i, t) in (420..540).step_by(15).enumerate() {
            add("R7", "WK", format!("p{i}"), 0, t);
        }
        for (i, t) in (600..900).step_by(30).enumerate() {
            add("R7", "WK", format!("m{i}"), 0, t);
        }
        add("R7", "WK", "back".into(), 1, 480);
        add("R7", "SAT", "sat".into(), 0, 480);
        add("R550", "WK", "r0".into(), 0, 400);
        add("R550", "WK", "r1".into(), 0, 1000);
        w("trips.txt", &trips);
        w("stop_times.txt", &st);

        let hw = read_headways(&dir).unwrap();
        let r7 = &hw["7"];
        assert_eq!(
            (r7.peak_headway_min, r7.midday_headway_min),
            (Some(15.0), Some(30.0))
        );
        assert_eq!((r7.trips, r7.span_hours), (18, 7.5));
        let r550 = &hw["550"];
        assert_eq!(
            (r550.peak_headway_min, r550.midday_headway_min),
            (None, None)
        );
        assert_eq!(r550.span_hours, 10.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
