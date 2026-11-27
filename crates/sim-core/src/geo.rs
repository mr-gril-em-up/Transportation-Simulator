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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_offset() {
        let (lon, lat) = offset(-97.74, 30.27, 3.0, 4.0);
        assert!((dist_km(-97.74, 30.27, lon, lat) - 5.0).abs() < 0.01);
    }
}
