//! Small geometry helpers. Equirectangular distance is accurate to well under
//! 1% at county and state scale, and much cheaper than haversine.

const KM_PER_DEG: f64 = 111.32;

/// Straight-line distance in km between two WGS84 points.
pub fn dist_km(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let mean_lat = ((lat1 + lat2) * 0.5).to_radians();
    let dx = (lon2 - lon1) * mean_lat.cos() * KM_PER_DEG;
    let dy = (lat2 - lat1) * KM_PER_DEG;
    (dx * dx + dy * dy).sqrt()
}

/// Offset a point by (east, north) km.
pub fn offset(lon: f64, lat: f64, east_km: f64, north_km: f64) -> (f64, f64) {
    let lat2 = lat + north_km / KM_PER_DEG;
    let lon2 = lon + east_km / (KM_PER_DEG * lat.to_radians().cos());
    (lon2, lat2)
}

/// Uniform lon/lat bucket grid over a fixed point set, for radius queries
/// that must return exactly what a brute-force scan over all points would
/// (SIM-20). `near` returns a superset of the points within the radius under
/// [`dist_km`], in ascending index order; callers apply their own exact
/// distance test, so results and their order match the O(N x M) loops.
pub struct PointGrid {
    cell_deg: f64,
    cells: std::collections::HashMap<(i32, i32), Vec<u32>>,
    n: usize,
}

impl PointGrid {
    /// `cell_km` sets the bucket size; about the typical query radius works well.
    pub fn new(points: impl IntoIterator<Item = (f64, f64)>, cell_km: f64) -> PointGrid {
        let cell_deg = (cell_km / KM_PER_DEG).max(1e-6);
        let mut cells: std::collections::HashMap<(i32, i32), Vec<u32>> = Default::default();
        let mut n = 0;
        for (i, (lon, lat)) in points.into_iter().enumerate() {
            let key = (
                (lon / cell_deg).floor() as i32,
                (lat / cell_deg).floor() as i32,
            );
            cells.entry(key).or_default().push(i as u32);
            n = i + 1;
        }
        PointGrid { cell_deg, cells, n }
    }

    /// Fill `out` with every point that may lie within `radius_km` of
    /// (lon, lat), sorted by index. Conservative: the longitude span uses the
    /// smallest cosine any point inside the latitude span can produce.
    pub fn near(&self, lon: f64, lat: f64, radius_km: f64, out: &mut Vec<u32>) {
        out.clear();
        let r = radius_km.max(0.0) * (1.0 + 1e-9) + 1e-9;
        let dlat = r / KM_PER_DEG;
        let worst = (lat.abs() + dlat).min(89.0).to_radians().cos();
        let dlon = r / (KM_PER_DEG * worst);
        let c = self.cell_deg;
        let (x0, x1) = (
            ((lon - dlon) / c).floor() as i64,
            ((lon + dlon) / c).floor() as i64,
        );
        let (y0, y1) = (
            ((lat - dlat) / c).floor() as i64,
            ((lat + dlat) / c).floor() as i64,
        );
        // A box bigger than the map: scanning the buckets is cheaper.
        if (x1 - x0 + 1) * (y1 - y0 + 1) > self.cells.len() as i64 {
            for v in self.cells.values() {
                out.extend_from_slice(v);
            }
        } else {
            for x in x0..=x1 {
                for y in y0..=y1 {
                    if let Some(v) = self.cells.get(&(x as i32, y as i32)) {
                        out.extend_from_slice(v);
                    }
                }
            }
        }
        out.sort_unstable();
    }

    /// Index of the nearest point (lowest index on ties), as a first-minimum
    /// scan over all points would return; None when the grid is empty.
    pub fn nearest(
        &self,
        lon: f64,
        lat: f64,
        pts: &[(f64, f64)],
        buf: &mut Vec<u32>,
    ) -> Option<u32> {
        if self.n == 0 {
            return None;
        }
        let mut r = self.cell_deg * KM_PER_DEG;
        loop {
            self.near(lon, lat, r, buf);
            let mut best = (f64::INFINITY, u32::MAX);
            for &i in buf.iter() {
                let (x, y) = pts[i as usize];
                let d = dist_km(lon, lat, x, y);
                if d < best.0 {
                    best = (d, i);
                }
            }
            // Every point within r is in buf, so a best inside r is the true one.
            if best.0 <= r || buf.len() == self.n {
                return Some(if best.1 == u32::MAX { 0 } else { best.1 });
            }
            r *= 2.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_matches_brute_force() {
        let mut s: u64 = 7;
        let mut rnd = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 11) as f64 / (1u64 << 53) as f64
        };
        let pts: Vec<(f64, f64)> = (0..2000)
            .map(|_| (-106.6 + 13.0 * rnd(), 25.8 + 10.7 * rnd()))
            .collect();
        let g = PointGrid::new(pts.iter().copied(), 1.5);
        let mut buf = Vec::new();
        let mut nb = Vec::new();
        for k in 0..300 {
            let (lon, lat) = (-106.6 + 13.0 * rnd(), 25.8 + 10.7 * rnd());
            let r = [0.4, 3.0, 40.0, 900.0][k % 4];
            g.near(lon, lat, r, &mut buf);
            let got: Vec<u32> = buf
                .iter()
                .copied()
                .filter(|&i| dist_km(lon, lat, pts[i as usize].0, pts[i as usize].1) <= r)
                .collect();
            let want: Vec<u32> = (0..pts.len() as u32)
                .filter(|&i| dist_km(lon, lat, pts[i as usize].0, pts[i as usize].1) <= r)
                .collect();
            assert_eq!(got, want);
            let mut best = (f64::INFINITY, 0u32);
            for (i, &(x, y)) in pts.iter().enumerate() {
                let d = dist_km(lon, lat, x, y);
                if d < best.0 {
                    best = (d, i as u32);
                }
            }
            assert_eq!(g.nearest(lon, lat, &pts, &mut nb), Some(best.1));
        }
    }

    #[test]
    fn round_trip_offset() {
        let (lon, lat) = offset(-97.74, 30.27, 3.0, 4.0);
        assert!((dist_km(-97.74, 30.27, lon, lat) - 5.0).abs() < 0.01);
    }
}
