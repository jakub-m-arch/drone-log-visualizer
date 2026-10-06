//! PUWG 1992 (EPSG:2180): transverse Mercator on GRS80, central meridian 19°E,
//! scale 0.9993, false easting 500 000 m, false northing −5 300 000 m.
//!
//! Krüger series to order n⁴ (sub-millimetre within Poland). Coordinates are
//! returned as `(easting, northing)`; note that EPSG:2180 itself names the
//! northing "x" and the easting "y".

const A: f64 = 6_378_137.0;
const F: f64 = 1.0 / 298.257_222_101;
const K0: f64 = 0.9993;
const LON0_DEG: f64 = 19.0;
const FE: f64 = 500_000.0;
const FN: f64 = -5_300_000.0;

struct Series {
    a_hat: f64,
    alpha: [f64; 4],
    beta: [f64; 4],
    delta: [f64; 4],
    e: f64,
}

fn series() -> Series {
    let n = F / (2.0 - F);
    let (n2, n3, n4) = (n * n, n * n * n, n * n * n * n);
    Series {
        a_hat: A / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0),
        alpha: [
            n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0,
            13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0,
            61.0 * n3 / 240.0 - 103.0 * n4 / 140.0,
            49561.0 * n4 / 161280.0,
        ],
        beta: [
            n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0 - n4 / 360.0,
            n2 / 48.0 + n3 / 15.0 - 437.0 * n4 / 1440.0,
            17.0 * n3 / 480.0 - 37.0 * n4 / 840.0,
            4397.0 * n4 / 161280.0,
        ],
        delta: [
            2.0 * n - 2.0 * n2 / 3.0 - 2.0 * n3 + 116.0 * n4 / 45.0,
            7.0 * n2 / 3.0 - 8.0 * n3 / 5.0 - 227.0 * n4 / 45.0,
            56.0 * n3 / 15.0 - 136.0 * n4 / 35.0,
            4279.0 * n4 / 630.0,
        ],
        e: (F * (2.0 - F)).sqrt(),
    }
}

/// WGS84/ETRS89 longitude, latitude (degrees) → (easting, northing) metres.
pub fn forward(lon: f64, lat: f64) -> (f64, f64) {
    let s = series();
    let phi = lat.to_radians();
    let dl = (lon - LON0_DEG).to_radians();
    let t = (phi.sin().atanh() - s.e * (s.e * phi.sin()).atanh()).sinh();
    let xi_p = t.atan2(dl.cos());
    let eta_p = (dl.sin() / (1.0 + t * t).sqrt()).atanh();
    let mut xi = xi_p;
    let mut eta = eta_p;
    for (j, a) in s.alpha.iter().enumerate() {
        let k = 2.0 * (j as f64 + 1.0);
        xi += a * (k * xi_p).sin() * (k * eta_p).cosh();
        eta += a * (k * xi_p).cos() * (k * eta_p).sinh();
    }
    (FE + K0 * s.a_hat * eta, FN + K0 * s.a_hat * xi)
}

/// (easting, northing) metres → longitude, latitude (degrees).
pub fn inverse(easting: f64, northing: f64) -> (f64, f64) {
    let s = series();
    let xi = (northing - FN) / (K0 * s.a_hat);
    let eta = (easting - FE) / (K0 * s.a_hat);
    let mut xi_p = xi;
    let mut eta_p = eta;
    for (j, b) in s.beta.iter().enumerate() {
        let k = 2.0 * (j as f64 + 1.0);
        xi_p -= b * (k * xi).sin() * (k * eta).cosh();
        eta_p -= b * (k * xi).cos() * (k * eta).sinh();
    }
    let chi = (xi_p.sin() / eta_p.cosh()).asin();
    let mut phi = chi;
    for (j, d) in s.delta.iter().enumerate() {
        phi += d * (2.0 * (j as f64 + 1.0) * chi).sin();
    }
    let lon = LON0_DEG + eta_p.sinh().atan2(xi_p.cos()).to_degrees();
    (lon, phi.to_degrees())
}

/// Rough bounding box of Poland (with a margin), to decide whether GUGiK data
/// can cover a flight.
pub fn in_poland(lon: f64, lat: f64) -> bool {
    (13.9..=24.4).contains(&lon) && (48.9..=55.1).contains(&lat)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference values from PROJ (pyproj, EPSG:4326 → EPSG:2180, xy order).
    const REF: [(f64, f64, f64, f64); 5] = [
        (19.0, 52.0, 500000.000, 459309.209),
        (21.0122, 52.2297, 637382.204, 486757.209),
        (14.2, 54.8, 191639.404, 781278.532),
        (24.1, 49.1, 872097.678, 149472.961),
        (16.9, 52.4, 357173.360, 505860.686),
    ];

    #[test]
    fn forward_matches_proj() {
        for (lon, lat, e, n) in REF {
            let (x, y) = forward(lon, lat);
            assert!(
                (x - e).abs() < 0.005 && (y - n).abs() < 0.005,
                "{lon},{lat}: {x},{y}"
            );
        }
    }

    #[test]
    fn inverse_round_trips() {
        for (lon, lat, e, n) in REF {
            let (lo, la) = inverse(e, n);
            assert!(
                (lo - lon).abs() < 1e-7 && (la - lat).abs() < 1e-7,
                "{lo},{la}"
            );
        }
    }

    #[test]
    fn poland_bbox() {
        assert!(in_poland(21.01, 52.23));
        assert!(!in_poland(13.4, 52.5)); // Berlin
        assert!(!in_poland(-122.0, 37.0));
    }
}
