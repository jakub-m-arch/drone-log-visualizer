//! Builder for small, synthetic DJI flight logs.
//!
//! Real flight logs contain GPS positions and serial numbers, so they cannot be
//! committed to an open-source repository. This module writes a minimal but
//! byte-exact unencrypted log (format version 6) that `dji-log-parser` reads like
//! a real one. It is used by the unit/integration tests and by the
//! `gen-sample-log` binary, which produces a file you can drag into the UI.

use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockEncryptMut, KeyIvInit};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as Base64;
use dji_log_parser::keychain::{FeaturePoint, KeychainFeaturePoint};
use std::collections::HashMap;
use std::f64::consts::PI;

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;

const VERSION: u8 = 6;
const PREFIX_SIZE: usize = 100;

const REC_OSD: u8 = 1;
const REC_HOME: u8 = 2;
const REC_GIMBAL: u8 = 3;
const REC_CUSTOM: u8 = 5;
const REC_CENTER_BATTERY: u8 = 7;
const REC_APP_TIP: u8 = 9;
const REC_APP_WARN: u8 = 10;
const REC_OFDM: u8 = 49;
const REC_CAMERA: u8 = 25;

/// Product type byte for "Mavic Pro" (3 battery cells).
const PRODUCT_MAVIC_PRO: u8 = 13;
/// Flight mode byte for "GPSAtti" (the normal P-GPS mode).
const FLIGHT_MODE_GPS_ATTI: u8 = 6;

/// One sample of the synthetic flight, in human units.
#[derive(Debug, Clone)]
pub struct SynthSample {
    pub timestamp_ms: i64,
    pub fly_time_s: f32,
    pub lat: f64,
    pub lon: f64,
    pub height_m: f32,
    pub speed_x: f32,
    pub speed_y: f32,
    pub speed_z: f32,
    pub yaw_deg: f32,
    pub gps_num: u8,
    pub battery_pct: u8,
    pub battery_mv: u16,
    pub rc_signal: u8,
    pub on_ground: bool,
    pub tip: Option<String>,
    pub warn: Option<String>,
    pub gimbal_pitch_deg: f32,
    pub photo: bool,
    pub recording: bool,
}

#[derive(Debug, Clone)]
pub struct SynthFlight {
    pub aircraft_name: String,
    /// Street and city written to the log details.
    pub street: String,
    pub city: String,
    pub aircraft_sn: String,
    pub start_ms: i64,
    pub home_lat: f64,
    pub home_lon: f64,
    pub home_alt_m: f32,
    pub samples: Vec<SynthSample>,
    /// Values written to the (unencrypted) log details section.
    pub details_distance_m: f32,
    pub details_max_height_m: f32,
}

impl SynthFlight {
    /// A ~60 s flight: climb to 40 m, fly a 120 m out-and-back leg east, land.
    /// Sampled at 10 Hz like real OSD records.
    pub fn demo() -> Self {
        let home_lat: f64 = 52.2297;
        let home_lon: f64 = 21.0122;
        let start_ms = 1_717_236_000_000; // 2024-06-01T10:00:00Z
        let n = 600;
        let mut samples = Vec::with_capacity(n);
        let m_per_deg_lon = 111_320.0 * home_lat.to_radians().cos();

        for i in 0..n {
            let t = i as f32 / 10.0;
            // Phases: 0-5 s ground, 5-15 climb, 15-30 out, 30-45 back, 45-55 descent, 55-60 ground.
            let (height, east, vz, vx) = match t {
                t if t < 5.0 => (0.0, 0.0, 0.0, 0.0),
                t if t < 15.0 => ((t - 5.0) * 4.0, 0.0, 4.0, 0.0),
                t if t < 30.0 => (40.0, (t - 15.0) * 8.0, 0.0, 8.0),
                t if t < 45.0 => (40.0, 120.0 - (t - 30.0) * 8.0, 0.0, -8.0),
                t if t < 55.0 => (40.0 - (t - 45.0) * 4.0, 0.0, -4.0, 0.0),
                _ => (0.0, 0.0, 0.0, 0.0),
            };
            let on_ground = !(5.0..55.0).contains(&t);
            let tip = match i {
                50 => Some("Takeoff".to_string()),
                300 => Some("Returning to home point".to_string()),
                _ => None,
            };
            let warn = match i {
                200 => Some("Strong wind. Fly with caution".to_string()),
                // Repeated warning that should be de-duplicated.
                201 => Some("Strong wind. Fly with caution".to_string()),
                420 => Some("Low battery".to_string()),
                _ => None,
            };
            samples.push(SynthSample {
                timestamp_ms: start_ms + i as i64 * 100,
                fly_time_s: if t < 5.0 { 0.0 } else { t - 5.0 },
                lat: home_lat,
                lon: home_lon + east as f64 / m_per_deg_lon,
                height_m: height,
                // DJI speed_x is north, speed_y is east.
                speed_x: 0.0,
                speed_y: vx,
                // DJI reports vertical speed positive downwards.
                speed_z: -vz,
                yaw_deg: if vx < 0.0 { -90.0 } else { 90.0 },
                gps_num: 14 + (i % 5) as u8,
                battery_pct: 95 - (i / 20) as u8,
                battery_mv: 12_600 - (i as u16) * 2,
                rc_signal: 100 - (east / 4.0) as u8,
                on_ground,
                tip,
                warn,
                // Look down for the first pass, straight down over the far end.
                gimbal_pitch_deg: match t {
                    t if on_ground || t < 15.0 => 0.0,
                    t if (28.0..33.0).contains(&t) => -90.0,
                    _ => -30.0,
                },
                // One photo spanning two samples at 25 s, another at 35 s.
                photo: matches!(i, 250 | 251 | 350),
                // Video from 15 s to 30 s.
                recording: (150..300).contains(&i),
            });
        }

        SynthFlight {
            aircraft_name: "Test Mavic".into(),
            street: "Main".into(),
            city: "Warsaw".into(),
            aircraft_sn: "SYNTH0001".into(),
            start_ms,
            home_lat,
            home_lon,
            home_alt_m: 100.0,
            samples,
            details_distance_m: 240.0,
            details_max_height_m: 40.0,
        }
    }

    /// One of three scenic flights over Kazimierz Dolny (Vistula riverside,
    /// market square, castle hill) on different days, used for the README
    /// screenshots and for trying the viewer. `variant` is taken modulo 3:
    /// 0 orbits the castle hill, 1 follows the river, 2 circles the market
    /// square. All stay within 450 m of take-off and below 120 m.
    pub fn showcase(variant: u8) -> Self {
        use Leg::*;
        // Take-off on the riverside boulevard.
        let (home_lat, home_lon) = (51.3242, 21.9463);
        let (rynek, castle) = ((74.0, -236.0), (282.0, -69.0));
        let (legs, start_ms, wind_at): (Vec<Leg>, i64, f64) = match variant % 3 {
            0 => (
                vec![
                    To(0.0, 0.0, 60.0, 3.0),
                    To(rynek.0, rynek.1, 60.0, 8.0),
                    Hover(6.0, true),
                    To(200.0, -150.0, 85.0, 7.0),
                    Orbit(castle.0, castle.1, 90.0, 95.0, 1.25, 6.0),
                    To(350.0, 150.0, 110.0, 9.0),
                    Hover(5.0, true),
                    To(0.0, 0.0, 40.0, 10.0),
                    To(0.0, 0.0, 0.0, 2.5),
                ],
                1_747_465_200_000, // 2025-05-17 07:00 UTC
                0.45,
            ),
            1 => (
                vec![
                    To(0.0, 0.0, 45.0, 3.0),
                    To(-60.0, 420.0, 50.0, 9.0),
                    Hover(4.0, true),
                    To(-30.0, -380.0, 70.0, 12.0),
                    Hover(4.0, true),
                    To(0.0, 0.0, 30.0, 9.0),
                    To(0.0, 0.0, 0.0, 2.0),
                ],
                1_749_394_800_000, // 2025-06-08 15:00 UTC
                0.6,
            ),
            _ => (
                vec![
                    To(0.0, 0.0, 50.0, 3.0),
                    To(rynek.0, rynek.1 + 70.0, 50.0, 7.0),
                    Orbit(rynek.0, rynek.1, 70.0, 50.0, 2.0, 5.0),
                    To(rynek.0, rynek.1, 70.0, 4.0),
                    Hover(5.0, true),
                    To(0.0, 0.0, 25.0, 8.0),
                    To(0.0, 0.0, 0.0, 2.0),
                ],
                1_755_968_400_000, // 2025-08-23 17:00 UTC
                0.3,
            ),
        };
        let mut f = Self::from_legs(&legs, home_lat, home_lon, start_ms, u64::from(variant));
        f.aircraft_name = "Demo Mavic".into();
        f.aircraft_sn = "DEMO0001".into();
        f.street = "Bulwar".into();
        f.city = "Kazimierz Dolny".into();
        f.home_alt_m = 120.0;
        let n = f.samples.len();
        let w = (n as f64 * wind_at) as usize;
        f.samples[w].warn = Some("Strong wind. Fly with caution".into());
        f
    }

    /// Builds a flight from route legs in local metres (east, north, height
    /// above take-off), sampled at 10 Hz, with 5 s on the ground at each end.
    fn from_legs(legs: &[Leg], home_lat: f64, home_lon: f64, start_ms: i64, seed: u64) -> Self {
        const DT: f64 = 0.1;
        // Dense path of (east, north, height, photo spot).
        let mut path: Vec<(f64, f64, f64, bool)> = vec![(0.0, 0.0, 0.0, false); 50];
        let mut cur = (0.0, 0.0, 0.0);
        for leg in legs {
            match *leg {
                Leg::To(x, y, h, v) => {
                    let len =
                        ((x - cur.0).powi(2) + (y - cur.1).powi(2) + (h - cur.2).powi(2)).sqrt();
                    let steps = ((len / v) / DT).ceil().max(1.0) as usize;
                    for k in 1..=steps {
                        let a = k as f64 / steps as f64;
                        path.push((
                            cur.0 + (x - cur.0) * a,
                            cur.1 + (y - cur.1) * a,
                            cur.2 + (h - cur.2) * a,
                            false,
                        ));
                    }
                    cur = (x, y, h);
                }
                Leg::Hover(secs, photo) => {
                    let steps = (secs / DT) as usize;
                    for k in 0..steps {
                        path.push((
                            cur.0,
                            cur.1,
                            cur.2,
                            photo && (k == steps / 3 || k == 2 * steps / 3),
                        ));
                    }
                }
                Leg::Orbit(cx, cy, r, h, turns, v) => {
                    let a0 = (cur.1 - cy).atan2(cur.0 - cx);
                    let total = turns * 2.0 * PI;
                    let steps = ((total * r / v) / DT).ceil() as usize;
                    let start = cur;
                    for k in 1..=steps {
                        let a = k as f64 / steps as f64;
                        let ang = a0 + total * a;
                        // Blend from the entry point onto the circle.
                        let blend = (a * 8.0).min(1.0);
                        let (ox, oy) = (cx + r * ang.cos(), cy + r * ang.sin());
                        path.push((
                            start.0 + (ox - start.0) * blend,
                            start.1 + (oy - start.1) * blend,
                            start.2 + (h - start.2) * blend,
                            false,
                        ));
                    }
                    let last = *path.last().expect("orbit has steps");
                    cur = (last.0, last.1, last.2);
                }
            }
        }
        path.extend(std::iter::repeat_n((0.0, 0.0, 0.0, false), 50));

        // Smooth corners, take-off and landing over ±1.5 s.
        let raw = path.clone();
        let half = 15usize;
        for (i, p) in path.iter_mut().enumerate() {
            let (a, b) = (i.saturating_sub(half), (i + half).min(raw.len() - 1));
            let m = (b - a + 1) as f64;
            let sum = raw[a..=b]
                .iter()
                .fold((0.0, 0.0, 0.0), |s, q| (s.0 + q.0, s.1 + q.1, s.2 + q.2));
            let h = sum.2 / m;
            (p.0, p.1, p.2) = (sum.0 / m, sum.1 / m, if h < 0.05 { 0.0 } else { h });
        }

        let m_per_deg_lon = 111_320.0 * home_lat.to_radians().cos();
        // Slow, smooth variation (in -1..1) so the charts look like real data.
        let phase = seed as f64 * 1.7;
        let wobble = |t: f64, period: f64| {
            ((t / period + phase) * 2.0 * PI).sin() * 0.7
                + ((t / (period * 0.37) + phase * 2.3) * 2.0 * PI).sin() * 0.3
        };
        let n = path.len();
        let airborne = |i: usize| path[i].2 > 0.05;
        let first_air = (0..n).find(|&i| airborne(i)).unwrap_or(0);
        let last_air = (0..n).rev().find(|&i| airborne(i)).unwrap_or(n - 1);
        let mut samples = Vec::with_capacity(n);
        let mut yaw = 0.0f64;
        for i in 0..n {
            let (x, y, h, photo_spot) = path[i];
            let (px, py, ph, _) = path[i.saturating_sub(1)];
            let (vx, vy, vz) = ((x - px) / DT, (y - py) / DT, (h - ph) / DT);
            if vx.hypot(vy) > 0.5 {
                yaw = vx.atan2(vy).to_degrees();
            }
            let dist = x.hypot(y);
            let t = i as f64 * DT;
            let on_ground = !(first_air..=last_air).contains(&i);
            let pct = (97.0 - t * 0.14).max(10.0);
            let recording = h > 20.0 && (i / 600) % 2 == 0;
            samples.push(SynthSample {
                timestamp_ms: start_ms + (t * 1000.0) as i64,
                fly_time_s: if i < first_air {
                    0.0
                } else {
                    ((i - first_air) as f64 * DT) as f32
                },
                lat: home_lat + y / 111_320.0,
                lon: home_lon + x / m_per_deg_lon,
                height_m: h as f32,
                speed_x: vy as f32,
                speed_y: vx as f32,
                speed_z: -vz as f32,
                yaw_deg: yaw as f32,
                gps_num: (19.5 + wobble(t, 90.0) * 2.5).round() as u8,
                battery_pct: pct as u8,
                // Voltage sags a little under load in climbs and fast legs.
                battery_mv: (3.0 * (3550.0 + 6.5 * pct)
                    - if on_ground {
                        0.0
                    } else {
                        60.0 + 25.0 * vz.max(0.0) + 4.0 * vx.hypot(vy)
                    }) as u16,
                rc_signal: (100.0 - dist / 9.0 + wobble(t, 25.0) * 4.0).clamp(20.0, 100.0) as u8,
                on_ground,
                tip: None,
                warn: None,
                gimbal_pitch_deg: match () {
                    _ if on_ground || h < 10.0 => 0.0,
                    _ if photo_spot || vx.hypot(vy) < 0.3 => -60.0,
                    _ => (-25.0 + wobble(t, 40.0) * 5.0) as f32,
                },
                photo: photo_spot,
                recording,
            });
        }
        let distance: f64 = path
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .sum();
        let max_h = path.iter().map(|p| p.2).fold(0.0, f64::max);
        SynthFlight {
            aircraft_name: String::new(),
            aircraft_sn: String::new(),
            street: String::new(),
            city: String::new(),
            start_ms,
            home_lat,
            home_lon,
            home_alt_m: 0.0,
            samples,
            details_distance_m: distance as f32,
            details_max_height_m: max_h as f32,
        }
    }

    /// Serializes the flight as an unencrypted log (format version 6).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = RecordWriter::raw();
        self.write_records(&mut w);
        let records = w.out;
        let detail_offset = (PREFIX_SIZE + records.len()) as u64;
        let details = self.details_payload();

        let mut out = prefix(VERSION, detail_offset, details.len() as u16);
        out.extend_from_slice(&records);
        out.extend_from_slice(&details);
        out
    }

    /// Serializes the flight as an encrypted log (format version 14): records
    /// are XOR-obfuscated and AES-256-CBC encrypted with `keys`, exactly the
    /// way `dji-log-parser` expects to decrypt them.
    pub fn to_encrypted_bytes(&self, keys: &SynthKeys) -> Vec<u8> {
        let mut w = RecordWriter::encrypted(keys.clone());
        self.write_records(&mut w);
        let records = w.out;

        // Auxiliary "info" block: XOR-encoded details (record type 0).
        let details = self.details_payload();
        let mut info = vec![1u8]; // version_data
        info.extend_from_slice(&(details.len() as u16).to_le_bytes());
        info.extend_from_slice(&details);
        info.extend_from_slice(&0u16.to_le_bytes()); // no signature
        let mut aux = vec![0u8];
        let seed = 0x42;
        aux.extend_from_slice(&((info.len() + 1) as u16).to_le_bytes());
        aux.push(seed);
        aux.extend_from_slice(&xor_encode(&info, seed, 0));
        // Auxiliary "version" block: keychain API version + department (DJI Fly).
        aux.push(1);
        aux.extend_from_slice(&3u16.to_le_bytes());
        aux.extend_from_slice(&1u16.to_le_bytes());
        aux.push(3);

        let records_offset = (PREFIX_SIZE + aux.len()) as u64;
        let mut out = prefix(ENCRYPTED_VERSION, records_offset, aux.len() as u16);
        out.extend_from_slice(&aux);
        out.extend_from_slice(&records);
        out
    }

    fn write_records(&self, w: &mut RecordWriter) {
        for (i, s) in self.samples.iter().enumerate() {
            // As in real logs, an OSD record opens a frame and the records that
            // follow it (until the next OSD) belong to that frame.
            w.push(REC_OSD, &osd_payload(s));
            if i % 10 == 0 {
                w.push(REC_HOME, &self.home_payload());
                w.push(REC_CENTER_BATTERY, &battery_payload(s));
            }
            w.push(REC_CUSTOM, &custom_payload(s));
            w.push(REC_GIMBAL, &gimbal_payload(s));
            w.push(REC_CAMERA, &camera_payload(s));
            w.push(REC_OFDM, &[s.rc_signal.min(127) | 0x80]);
            w.push(REC_OFDM, &[s.rc_signal.min(127)]);
            if let Some(tip) = &s.tip {
                w.push(REC_APP_TIP, tip.as_bytes());
            }
            if let Some(warn) = &s.warn {
                w.push(REC_APP_WARN, warn.as_bytes());
            }
        }
        // The parser emits a frame when the *next* OSD record starts, so a
        // trailing OSD record flushes the last real sample.
        if let Some(last) = self.samples.last() {
            w.push(REC_OSD, &osd_payload(last));
        }
    }

    fn home_payload(&self) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&self.home_lon.to_radians().to_le_bytes());
        p.extend_from_slice(&self.home_lat.to_radians().to_le_bytes());
        p.extend_from_slice(&(self.home_alt_m * 10.0).to_le_bytes());
        p.push(0x01); // is_home_record
        p.push(0x00);
        p.extend_from_slice(&30u16.to_le_bytes()); // go home height
        p.extend_from_slice(&0i16.to_le_bytes());
        p.extend_from_slice(&[0, 0]);
        p.extend_from_slice(&0u16.to_le_bytes());
        p.extend_from_slice(&1u16.to_le_bytes());
        p
    }

    fn details_payload(&self) -> Vec<u8> {
        let duration_ms = self.samples.len() as i32 * 100;
        let mut p = Vec::new();
        for s in ["", &self.street, &self.city, "PL"] {
            p.extend_from_slice(&fixed_str(s, 20));
        }
        p.extend_from_slice(&[0, 1, 0]); // favorite, new, needs upload
        p.extend_from_slice(&(self.samples.len() as i32).to_le_bytes());
        p.extend_from_slice(&0i32.to_le_bytes()); // checksum
        p.extend_from_slice(&self.start_ms.to_le_bytes());
        p.extend_from_slice(&self.home_lon.to_le_bytes());
        p.extend_from_slice(&self.home_lat.to_le_bytes());
        p.extend_from_slice(&self.details_distance_m.to_le_bytes());
        p.extend_from_slice(&duration_ms.to_le_bytes());
        p.extend_from_slice(&self.details_max_height_m.to_le_bytes());
        let max_h = self
            .samples
            .iter()
            .map(|s| s.speed_x.hypot(s.speed_y))
            .fold(0.0f32, f32::max);
        let max_v = self
            .samples
            .iter()
            .map(|s| s.speed_z.abs())
            .fold(0.0f32, f32::max);
        let photos = self
            .samples
            .windows(2)
            .filter(|w| w[1].photo && !w[0].photo)
            .count() as i32;
        p.extend_from_slice(&max_h.to_le_bytes()); // max horizontal speed
        p.extend_from_slice(&max_v.to_le_bytes()); // max vertical speed
        p.extend_from_slice(&photos.to_le_bytes()); // photos
        p.extend_from_slice(&0i64.to_le_bytes()); // video time
        p.extend_from_slice(&[0u8; 4 * 4 * 2]); // moment pic buffer lengths
        p.extend_from_slice(&[0u8; 8 * 4 * 2]); // moment pic coordinates
        p.extend_from_slice(&0i64.to_le_bytes()); // analysis offset
        p.extend_from_slice(&[0u8; 16]); // md5
        p.extend_from_slice(&self.home_alt_m.to_le_bytes()); // take off altitude
        p.push(PRODUCT_MAVIC_PRO);
        p.extend_from_slice(&0i64.to_le_bytes()); // activation timestamp
        p.extend_from_slice(&fixed_str(&self.aircraft_name, 32));
        p.extend_from_slice(&fixed_str(&self.aircraft_sn, 16));
        p.extend_from_slice(&fixed_str("CAM0001", 16));
        p.extend_from_slice(&fixed_str("RC0001", 16));
        p.extend_from_slice(&fixed_str("BAT0001", 16));
        p.push(1); // iOS
        p.extend_from_slice(&[4, 3, 0]);
        p.resize(p.len().max(400), 0);
        p
    }
}

/// File names of the showcase flights (`SynthFlight::showcase(0..3)`), as
/// committed in `samples/`, in DJI's naming scheme (local take-off time).
pub const SHOWCASE_FILES: [&str; 3] = [
    "DJIFlightRecord_2025-05-17_[09-00-00].txt",
    "DJIFlightRecord_2025-06-08_[17-00-00].txt",
    "DJIFlightRecord_2025-08-23_[19-00-00].txt",
];

/// A piece of a synthetic route, in local metres relative to take-off.
#[derive(Debug, Clone, Copy)]
enum Leg {
    /// Fly straight to (east, north, height) at the given speed (m/s).
    To(f64, f64, f64, f64),
    /// Hover for the given seconds, taking two photos if `true`.
    Hover(f64, bool),
    /// Circle (centre east, centre north, radius, height, turns, speed).
    Orbit(f64, f64, f64, f64, f64, f64),
}

fn prefix(version: u8, detail_offset: u64, detail_length: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(PREFIX_SIZE);
    out.extend_from_slice(&detail_offset.to_le_bytes());
    out.extend_from_slice(&detail_length.to_le_bytes());
    out.push(version);
    out.push(0); // unknown
    out.extend_from_slice(&0u64.to_le_bytes()); // encrypt magic version
    out.resize(PREFIX_SIZE, 0);
    out
}

/// AES keys per feature point for encrypted synthetic logs.
#[derive(Debug, Clone)]
pub struct SynthKeys {
    keys: HashMap<FeaturePoint, ([u8; 32], [u8; 16])>,
}

impl SynthKeys {
    /// Deterministic keys derived from `seed`; different seeds give different keys.
    pub fn new(seed: u8) -> Self {
        let keys = ENCRYPTED_RECORD_TYPES
            .iter()
            .map(|&t| FeaturePoint::from_record_type(t, ENCRYPTED_VERSION))
            .filter(|fp| *fp != FeaturePoint::PlaintextFeature)
            .map(|fp| {
                let n = fp as u8;
                let key = std::array::from_fn(|i| seed.wrapping_mul(31).wrapping_add(n ^ i as u8));
                let iv = std::array::from_fn(|i| seed.wrapping_add(n).wrapping_mul(7) ^ i as u8);
                (fp, (key, iv))
            })
            .collect();
        SynthKeys { keys }
    }

    /// The keychains as the DJI API would return them (`data` field).
    pub fn api_keychains(&self) -> Vec<Vec<KeychainFeaturePoint>> {
        vec![
            self.keys
                .iter()
                .map(|(fp, (key, iv))| KeychainFeaturePoint {
                    feature_point: *fp,
                    aes_key: Base64.encode(key),
                    aes_iv: Base64.encode(iv),
                })
                .collect(),
        ]
    }
}

const ENCRYPTED_VERSION: u8 = 14;
const ENCRYPTED_RECORD_TYPES: [u8; 9] = [
    REC_OSD,
    REC_HOME,
    REC_GIMBAL,
    REC_CAMERA,
    REC_CUSTOM,
    REC_CENTER_BATTERY,
    REC_APP_TIP,
    REC_APP_WARN,
    REC_OFDM,
];

struct RecordWriter {
    out: Vec<u8>,
    keys: Option<SynthKeys>,
    /// CBC chaining: each record's IV is the last block of the previous
    /// ciphertext for the same feature point.
    ivs: HashMap<FeaturePoint, [u8; 16]>,
    counter: u8,
}

impl RecordWriter {
    fn raw() -> Self {
        RecordWriter {
            out: Vec::new(),
            keys: None,
            ivs: HashMap::new(),
            counter: 0,
        }
    }

    fn encrypted(keys: SynthKeys) -> Self {
        let ivs = keys.keys.iter().map(|(fp, (_, iv))| (*fp, *iv)).collect();
        RecordWriter {
            out: Vec::new(),
            keys: Some(keys),
            ivs,
            counter: 0,
        }
    }

    fn push(&mut self, kind: u8, payload: &[u8]) {
        let Some(keys) = &self.keys else {
            // v6: [type][u8 length][payload][0xFF]
            assert!(payload.len() <= u8::MAX as usize, "record payload too long");
            self.out.push(kind);
            self.out.push(payload.len() as u8);
            self.out.extend_from_slice(payload);
            self.out.push(0xFF);
            return;
        };

        // v13+: [type][u16 length][seed][xor(body)][pad][0xFF]; length counts seed and pad.
        self.counter = self.counter.wrapping_add(37);
        let seed = self.counter;
        let fp = FeaturePoint::from_record_type(kind, ENCRYPTED_VERSION);
        let body = match keys.keys.get(&fp) {
            Some((key, _)) => {
                let iv = self.ivs[&fp];
                let mut buf = payload.to_vec();
                let len = buf.len();
                buf.resize(len + 16, 0);
                let ct = Aes256CbcEnc::new(key.into(), &iv.into())
                    .encrypt_padded_mut::<Pkcs7>(&mut buf, len)
                    .expect("buffer has room for padding")
                    .to_vec();
                self.ivs.insert(fp, ct[ct.len() - 16..].try_into().unwrap());
                ct
            }
            None => payload.to_vec(),
        };
        self.out.push(kind);
        self.out
            .extend_from_slice(&((body.len() + 2) as u16).to_le_bytes());
        self.out.push(seed);
        self.out.extend_from_slice(&xor_encode(&body, seed, kind));
        self.out.push(0x00); // trailing char, skipped by the parser's `pad_size_to`
        self.out.push(0xFF);
    }
}

/// Inverse of the parser's `XorDecoder` (XOR is symmetric).
fn xor_encode(data: &[u8], seed: u8, record_type: u8) -> Vec<u8> {
    let magic: u64 = 0x123456789ABCDEF0;
    let key = crc64::crc64(
        seed.wrapping_add(record_type) as u64,
        &magic.wrapping_mul(seed as u64).to_le_bytes(),
    )
    .to_le_bytes();
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % 8])
        .collect()
}

fn fixed_str(s: &str, len: usize) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.resize(len, 0);
    v
}

fn deci(v: f32) -> [u8; 2] {
    ((v * 10.0).round() as i16).to_le_bytes()
}

fn osd_payload(s: &SynthSample) -> Vec<u8> {
    let mut p = Vec::with_capacity(53);
    p.extend_from_slice(&(s.lon * PI / 180.0).to_le_bytes());
    p.extend_from_slice(&(s.lat * PI / 180.0).to_le_bytes());
    p.extend_from_slice(&deci(s.height_m));
    p.extend_from_slice(&deci(s.speed_x));
    p.extend_from_slice(&deci(s.speed_y));
    p.extend_from_slice(&deci(s.speed_z));
    p.extend_from_slice(&deci(0.0)); // pitch
    p.extend_from_slice(&deci(0.0)); // roll
    p.extend_from_slice(&deci(s.yaw_deg));
    p.push(FLIGHT_MODE_GPS_ATTI);
    p.push(0); // app command
    // bitpack2: ground_or_sky in bits 1-2 (0 ground, 2 sky), motor up in bit 3.
    p.push(if s.on_ground { 0x00 } else { (2 << 1) | 0x08 });
    p.push(0x80); // bitpack3: GPS valid
    p.push(5 << 2); // bitpack4: GPS level 5
    p.push(0); // bitpack5
    p.push(s.gps_num);
    p.push(0); // flight action
    p.push(0); // motor start failed cause
    p.push(0); // bitpack6
    p.push(s.battery_pct);
    p.push(0); // s_wave height
    p.extend_from_slice(&((s.fly_time_s * 10.0).round() as u16).to_le_bytes());
    p.push(0); // motor revolution
    p.extend_from_slice(&0u16.to_le_bytes());
    p.push(0); // version c
    p.push(0); // drone type
    p.push(0); // imu init fail reason
    p.resize(53, 0);
    p
}

fn gimbal_payload(s: &SynthSample) -> Vec<u8> {
    let mut p = Vec::with_capacity(16);
    p.extend_from_slice(&deci(s.gimbal_pitch_deg));
    p.extend_from_slice(&deci(0.0)); // roll
    p.extend_from_slice(&deci(s.yaw_deg));
    p.push(0); // mode / reset
    p.push(0); // roll adjust
    p.extend_from_slice(&deci(0.0)); // yaw angle
    p.push(0); // limits / calibration flags
    p.push(0); // version / double click
    p.resize(16, 0);
    p
}

fn camera_payload(s: &SynthSample) -> Vec<u8> {
    let mut p = vec![0u8; 32];
    // bitpack1: connected, single photo (bits 3-5 == 1), recording (bits 6-7).
    p[0] = 0x01 | if s.photo { 0x08 } else { 0 } | if s.recording { 0x40 } else { 0 };
    p[1] = 0x02; // SD card inserted
    p
}

fn custom_payload(s: &SynthSample) -> Vec<u8> {
    let mut p = Vec::with_capacity(18);
    p.push(0);
    p.push(0);
    p.extend_from_slice(&0f32.to_le_bytes());
    p.extend_from_slice(&0f32.to_le_bytes());
    p.extend_from_slice(&s.timestamp_ms.to_le_bytes());
    p
}

fn battery_payload(s: &SynthSample) -> Vec<u8> {
    let mut p = Vec::with_capacity(32);
    p.push(s.battery_pct);
    p.extend_from_slice(&s.battery_mv.to_le_bytes());
    p.extend_from_slice(&3000u16.to_le_bytes());
    p.extend_from_slice(&3800u16.to_le_bytes());
    p.push(98); // life
    p.extend_from_slice(&42u16.to_le_bytes()); // discharges
    p.extend_from_slice(&0u32.to_le_bytes()); // error
    p.extend_from_slice(&(-5000i16).to_le_bytes()); // current mA
    let cell = s.battery_mv / 3;
    for i in 0..6 {
        p.extend_from_slice(&(if i < 3 { cell } else { 0 }).to_le_bytes());
    }
    p.extend_from_slice(&1u16.to_le_bytes());
    p.extend_from_slice(&0u16.to_le_bytes());
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use dji_log_parser::DJILog;

    /// `samples/` holds the showcase flights for trying the viewer; they must
    /// match the generator (regenerate with `gen-sample-log --showcase`).
    #[test]
    fn committed_samples_match_the_generator() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../samples");
        for (v, name) in SHOWCASE_FILES.iter().enumerate() {
            let bytes = std::fs::read(dir.join(name)).unwrap();
            assert!(
                bytes == SynthFlight::showcase(v as u8).to_bytes(),
                "{name} is out of date"
            );
        }
    }

    #[test]
    fn showcase_flights_parse_and_stay_near_takeoff() {
        for v in 0..3 {
            let synth = SynthFlight::showcase(v);
            let log = DJILog::from_bytes(synth.to_bytes()).unwrap();
            let frames = log.frames(None).unwrap();
            assert!(frames.len() > 1500, "variant {v}: {} frames", frames.len());
            let m_lon = 111_320.0 * synth.home_lat.to_radians().cos();
            for f in &frames {
                let (dx, dy) = (
                    (f.osd.longitude - synth.home_lon) * m_lon,
                    (f.osd.latitude - synth.home_lat) * 111_320.0,
                );
                assert!(dx.hypot(dy) < 450.0, "variant {v}");
                assert!((0.0..120.0).contains(&f.osd.height), "variant {v}");
            }
            assert!(frames.iter().any(|f| f.camera.is_photo));
        }
    }

    #[test]
    fn encrypted_log_decrypts_to_same_frames_as_raw() {
        let synth = SynthFlight::demo();
        let keys = SynthKeys::new(1);

        let raw = DJILog::from_bytes(synth.to_bytes()).unwrap();
        let enc = DJILog::from_bytes(synth.to_encrypted_bytes(&keys)).unwrap();
        assert_eq!(enc.version, 14);
        assert_eq!(enc.details.aircraft_name, "Test Mavic");
        assert!(matches!(
            enc.frames(None),
            Err(dji_log_parser::Error::KeychainRequired)
        ));
        let request = enc.keychains_request().unwrap();
        assert_eq!(request.department, 3);

        let a = raw.frames(None).unwrap();
        let b = enc.frames(Some(keys.api_keychains())).unwrap();
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.osd.latitude, y.osd.latitude);
            assert_eq!(x.osd.height, y.osd.height);
            assert_eq!(x.custom.date_time, y.custom.date_time);
            assert_eq!(x.battery.voltage, y.battery.voltage);
            assert_eq!(x.rc.uplink_signal, y.rc.uplink_signal);
            assert_eq!(x.app.warn, y.app.warn);
            assert_eq!(x.gimbal.pitch, y.gimbal.pitch);
            assert_eq!(x.camera.is_photo, y.camera.is_photo);
            assert_eq!(x.camera.is_video, y.camera.is_video);
        }
    }

    #[test]
    fn wrong_keys_do_not_decrypt() {
        let synth = SynthFlight::demo();
        let enc = DJILog::from_bytes(synth.to_encrypted_bytes(&SynthKeys::new(1))).unwrap();
        let frames = enc.frames(Some(SynthKeys::new(2).api_keychains())).unwrap();
        assert!(
            frames
                .iter()
                .all(|f| f.osd.latitude == 0.0 && f.osd.height == 0.0)
        );
    }
}
