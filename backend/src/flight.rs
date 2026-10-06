//! Mapping from `dji-log-parser` frames to the application's flight model.
//!
//! Everything here is pure (no I/O) so it can be unit tested in isolation.

use chrono::{DateTime, Datelike, Utc};
use dji_log_parser::frame::Frame;
use dji_log_parser::layout::details::Details;
use serde::Serialize;

/// Flight summary + metadata, stored in the `flights` table.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlightMeta {
    pub log_version: u8,
    pub encrypted: bool,
    pub aircraft_name: String,
    pub aircraft_sn: String,
    pub product_type: String,
    pub app_platform: String,
    pub app_version: String,
    pub start_time: Option<DateTime<Utc>>,
    pub duration_s: f64,
    pub distance_m: f64,
    pub max_height_m: f64,
    pub max_h_speed_ms: f64,
    pub max_v_speed_ms: f64,
    pub home_lat: Option<f64>,
    pub home_lon: Option<f64>,
    pub takeoff_lat: Option<f64>,
    pub takeoff_lon: Option<f64>,
    pub landing_lat: Option<f64>,
    pub landing_lon: Option<f64>,
    pub location: String,
    pub sample_count: usize,
}

/// One telemetry sample (one OSD frame, ~10 Hz).
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    /// Seconds since the first sample.
    pub t: f64,
    /// Absolute UTC timestamp in milliseconds, when the log carries one.
    pub timestamp_ms: Option<i64>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// Height above the take-off point, metres.
    pub height_m: f64,
    /// Altitude above sea level, metres.
    pub altitude_m: f64,
    /// Horizontal ground speed, m/s.
    pub h_speed_ms: f64,
    /// Vertical speed, m/s, positive = climbing.
    pub v_speed_ms: f64,
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    pub roll_deg: f64,
    pub battery_pct: Option<f64>,
    pub battery_v: Option<f64>,
    pub gps_sats: i64,
    pub rc_uplink_pct: Option<f64>,
    pub rc_downlink_pct: Option<f64>,
    pub flight_mode: String,
    pub is_flying: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EventLevel {
    Info,
    Warning,
}

impl EventLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            EventLevel::Info => "info",
            EventLevel::Warning => "warning",
        }
    }

    pub fn parse(s: &str) -> Self {
        if s == "warning" {
            EventLevel::Warning
        } else {
            EventLevel::Info
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub t: f64,
    pub level: EventLevel,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ParsedFlight {
    pub meta: FlightMeta,
    pub samples: Vec<Sample>,
    pub events: Vec<Event>,
}

/// Identical messages repeated within this window are reported once.
const EVENT_DEDUP_WINDOW_S: f64 = 5.0;
/// Position jumps faster than this between two samples are treated as GPS glitches.
const MAX_PLAUSIBLE_SPEED_MS: f64 = 100.0;
/// Sample period assumed when the log carries no usable clock.
const DEFAULT_SAMPLE_PERIOD_S: f64 = 0.1;

pub fn build_flight(version: u8, details: &Details, frames: &[Frame]) -> ParsedFlight {
    let times = time_axis(frames);
    let samples: Vec<Sample> = frames
        .iter()
        .zip(&times)
        .map(|(f, &(t, ts))| frame_to_sample(f, t, ts))
        .collect();
    let events = extract_events(frames, &samples);
    let meta = summarize(version, details, frames, &samples);
    ParsedFlight {
        meta,
        samples,
        events,
    }
}

fn frame_to_sample(f: &Frame, t: f64, timestamp_ms: Option<i64>) -> Sample {
    let (lat, lon) = valid_position(f.osd.latitude, f.osd.longitude);
    let battery_known = f.battery.voltage > 0.0 || f.battery.charge_level > 0;
    Sample {
        t,
        timestamp_ms,
        lat,
        lon,
        height_m: round(f.osd.height as f64, 2),
        altitude_m: round(f.osd.altitude as f64, 2),
        h_speed_ms: round((f.osd.x_speed as f64).hypot(f.osd.y_speed as f64), 2),
        // DJI uses a NED frame: positive z speed means descending.
        v_speed_ms: round(-(f.osd.z_speed as f64), 2),
        yaw_deg: round(f.osd.yaw as f64, 1),
        pitch_deg: round(f.osd.pitch as f64, 1),
        roll_deg: round(f.osd.roll as f64, 1),
        battery_pct: battery_known.then_some(f.battery.charge_level as f64),
        battery_v: (f.battery.voltage > 0.0).then(|| round(f.battery.voltage as f64, 3)),
        gps_sats: f.osd.gps_num as i64,
        rc_uplink_pct: f.rc.uplink_signal.map(f64::from),
        rc_downlink_pct: f.rc.downlink_signal.map(f64::from),
        flight_mode: f
            .osd
            .flyc_state
            .map(|m| format!("{m:?}"))
            .unwrap_or_default(),
        is_flying: !f.osd.is_on_ground || f.osd.height.abs() > 0.5,
    }
}

/// Returns `(lat, lon)` if the pair is a plausible GPS fix.
pub fn valid_position(lat: f64, lon: f64) -> (Option<f64>, Option<f64>) {
    let plausible = lat.is_finite()
        && lon.is_finite()
        && lat.abs() <= 90.0
        && lon.abs() <= 180.0
        && (lat.abs() > 1e-6 || lon.abs() > 1e-6);
    if plausible {
        (Some(lat), Some(lon))
    } else {
        (None, None)
    }
}

fn valid_timestamp(dt: &DateTime<Utc>) -> Option<i64> {
    (dt.year() > 2010 && dt.year() < 2100).then(|| dt.timestamp_millis())
}

/// Builds a monotonic time axis (seconds since first sample) for the frames.
///
/// The log's clock (`custom.date_time`) is preferred. It is often updated less
/// frequently than OSD frames, so runs of identical timestamps are spread
/// evenly between neighbouring distinct values. Without a usable clock the
/// nominal 10 Hz OSD rate is assumed.
pub fn time_axis(frames: &[Frame]) -> Vec<(f64, Option<i64>)> {
    let raw: Vec<Option<i64>> = frames
        .iter()
        .map(|f| valid_timestamp(&f.custom.date_time))
        .collect();
    let clock: Vec<i64> = raw.iter().flatten().copied().collect();
    let distinct = {
        let mut c = clock.clone();
        c.dedup();
        c.len()
    };
    let usable = clock.len() * 2 >= frames.len() && distinct >= 2;
    if !usable {
        return (0..frames.len())
            .map(|i| (round(i as f64 * DEFAULT_SAMPLE_PERIOD_S, 3), None))
            .collect();
    }

    // Forward-fill missing values, then force monotonicity.
    let mut filled = Vec::with_capacity(raw.len());
    let mut last = clock[0];
    for v in &raw {
        let v = v.unwrap_or(last).max(last);
        filled.push(v);
        last = v;
    }

    let mut ms: Vec<f64> = filled.iter().map(|&v| v as f64).collect();
    let mut i = 0;
    while i < filled.len() {
        let mut j = i;
        while j + 1 < filled.len() && filled[j + 1] == filled[i] {
            j += 1;
        }
        let run = j - i + 1;
        if run > 1 {
            let step = if j + 1 < filled.len() {
                (filled[j + 1] - filled[i]) as f64 / run as f64
            } else {
                DEFAULT_SAMPLE_PERIOD_S * 1000.0
            };
            for k in 0..run {
                ms[i + k] = filled[i] as f64 + step * k as f64;
            }
        }
        i = j + 1;
    }

    let t0 = ms[0];
    ms.iter()
        .map(|&v| (round((v - t0) / 1000.0, 3), Some(v.round() as i64)))
        .collect()
}

/// Turns the per-frame app tips/warnings into a de-duplicated event list,
/// and adds synthetic take-off / landing events.
pub fn extract_events(frames: &[Frame], samples: &[Sample]) -> Vec<Event> {
    let mut events: Vec<Event> = Vec::new();
    let mut last_seen: std::collections::HashMap<String, f64> = Default::default();

    let mut push = |events: &mut Vec<Event>, t: f64, level: EventLevel, msg: &str| {
        let msg = msg.trim();
        if msg.is_empty() {
            return;
        }
        let key = format!("{}|{}", level.as_str(), msg);
        if let Some(&prev) = last_seen.get(&key)
            && t - prev < EVENT_DEDUP_WINDOW_S
        {
            last_seen.insert(key, t);
            return;
        }
        last_seen.insert(key, t);
        events.push(Event {
            t,
            level,
            message: msg.to_string(),
        });
    };

    let mut was_flying = false;
    for (f, s) in frames.iter().zip(samples) {
        if s.is_flying && !was_flying {
            push(&mut events, s.t, EventLevel::Info, "Take-off");
        } else if !s.is_flying && was_flying {
            push(&mut events, s.t, EventLevel::Info, "Landing");
        }
        was_flying = s.is_flying;

        for msg in f.app.tip.split("; ") {
            push(&mut events, s.t, EventLevel::Info, msg);
        }
        for msg in f.app.warn.split("; ") {
            push(&mut events, s.t, EventLevel::Warning, msg);
        }
    }
    events
}

pub fn summarize(
    version: u8,
    details: &Details,
    frames: &[Frame],
    samples: &[Sample],
) -> FlightMeta {
    let positions: Vec<(f64, f64, f64)> = samples
        .iter()
        .filter_map(|s| Some((s.t, s.lat?, s.lon?)))
        .collect();

    let computed_distance = track_distance(&positions);
    let computed_max_height = samples.iter().map(|s| s.height_m).fold(0.0, f64::max);
    let computed_max_h = samples.iter().map(|s| s.h_speed_ms).fold(0.0, f64::max);
    let computed_max_v = samples
        .iter()
        .map(|s| s.v_speed_ms.abs())
        .fold(0.0, f64::max);
    let computed_duration = samples.last().map(|s| s.t).unwrap_or(0.0);

    let flying: Vec<&Sample> = samples
        .iter()
        .filter(|s| s.is_flying && s.lat.is_some())
        .collect();
    let first_pos = flying
        .first()
        .copied()
        .or_else(|| samples.iter().find(|s| s.lat.is_some()));
    let last_pos = flying
        .last()
        .copied()
        .or_else(|| samples.iter().rev().find(|s| s.lat.is_some()));

    let home = frames
        .iter()
        .rev()
        .map(|f| valid_position(f.home.latitude, f.home.longitude))
        .find(|(lat, _)| lat.is_some())
        .unwrap_or((None, None));

    let start_time = valid_timestamp(&details.start_time)
        .and_then(DateTime::from_timestamp_millis)
        .or_else(|| {
            samples
                .iter()
                .find_map(|s| s.timestamp_ms)
                .and_then(DateTime::from_timestamp_millis)
        });

    let location = [details.city.as_str(), details.area.as_str()]
        .iter()
        .filter(|s| !s.trim().is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(", ");

    let prefer = |from_details: f64, computed: f64| {
        if from_details.is_finite() && from_details > 0.0 {
            round(from_details, 2)
        } else {
            round(computed, 2)
        }
    };

    FlightMeta {
        log_version: version,
        encrypted: version >= 13,
        aircraft_name: details.aircraft_name.trim().to_string(),
        aircraft_sn: details.aircraft_sn.trim().to_string(),
        product_type: format!("{:?}", details.product_type),
        app_platform: format!("{:?}", details.app_platform),
        app_version: details.app_version.clone(),
        start_time,
        duration_s: prefer(details.total_time, computed_duration),
        distance_m: prefer(details.total_distance as f64, computed_distance),
        max_height_m: prefer(details.max_height as f64, computed_max_height),
        max_h_speed_ms: prefer(details.max_horizontal_speed as f64, computed_max_h),
        max_v_speed_ms: prefer(details.max_vertical_speed as f64, computed_max_v),
        home_lat: home.0,
        home_lon: home.1,
        takeoff_lat: first_pos.and_then(|s| s.lat),
        takeoff_lon: first_pos.and_then(|s| s.lon),
        landing_lat: last_pos.and_then(|s| s.lat),
        landing_lon: last_pos.and_then(|s| s.lon),
        location,
        sample_count: samples.len(),
    }
}

/// Great-circle distance in metres.
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_008.8;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

/// Sums distances between consecutive `(t, lat, lon)` points, skipping
/// implausible jumps (GPS glitches).
pub fn track_distance(points: &[(f64, f64, f64)]) -> f64 {
    points
        .windows(2)
        .map(|w| {
            let (t1, la1, lo1) = w[0];
            let (t2, la2, lo2) = w[1];
            let d = haversine_m(la1, lo1, la2, lo2);
            let dt = (t2 - t1).max(DEFAULT_SAMPLE_PERIOD_S);
            if d / dt > MAX_PLAUSIBLE_SPEED_MS {
                0.0
            } else {
                d
            }
        })
        .sum()
}

fn round(v: f64, decimals: i32) -> f64 {
    let f = 10f64.powi(decimals);
    (v * f).round() / f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::SynthFlight;
    use dji_log_parser::DJILog;

    fn frame_at(lat: f64, lon: f64, ts_ms: i64) -> Frame {
        let mut f = Frame::default();
        f.osd.latitude = lat;
        f.osd.longitude = lon;
        f.custom.date_time = DateTime::from_timestamp_millis(ts_ms).unwrap();
        f
    }

    #[test]
    fn haversine_known_distance() {
        // One degree of latitude is ~111.2 km.
        let d = haversine_m(0.0, 0.0, 1.0, 0.0);
        assert!((d - 111_195.0).abs() < 50.0, "{d}");
    }

    #[test]
    fn track_distance_skips_gps_glitches() {
        let pts = vec![
            (0.0, 52.0, 21.0),
            (0.1, 52.0, 21.00001), // ~0.7 m
            (0.2, 53.0, 21.00001), // 111 km jump in 0.1 s -> ignored
            (0.3, 52.0, 21.00002),
        ];
        let d = track_distance(&pts);
        assert!(d < 5.0, "{d}");
    }

    #[test]
    fn rejects_null_island_and_out_of_range() {
        assert_eq!(valid_position(0.0, 0.0), (None, None));
        assert_eq!(valid_position(91.0, 10.0), (None, None));
        assert_eq!(valid_position(f64::NAN, 10.0), (None, None));
        assert_eq!(valid_position(52.1, 21.0), (Some(52.1), Some(21.0)));
    }

    #[test]
    fn time_axis_spreads_repeated_timestamps() {
        let base = 1_717_236_000_000;
        let frames = vec![
            frame_at(52.0, 21.0, base),
            frame_at(52.0, 21.0, base),
            frame_at(52.0, 21.0, base + 1000),
            frame_at(52.0, 21.0, base + 1000),
        ];
        let t: Vec<f64> = time_axis(&frames).into_iter().map(|(t, _)| t).collect();
        assert_eq!(t, vec![0.0, 0.5, 1.0, 1.1]);
    }

    #[test]
    fn time_axis_falls_back_to_nominal_rate_without_clock() {
        let frames = vec![Frame::default(), Frame::default(), Frame::default()];
        let axis = time_axis(&frames);
        assert_eq!(axis, vec![(0.0, None), (0.1, None), (0.2, None)]);
    }

    #[test]
    fn time_axis_is_monotonic_when_clock_goes_backwards() {
        let base = 1_717_236_000_000;
        let frames = vec![
            frame_at(0.0, 0.0, base),
            frame_at(0.0, 0.0, base + 200),
            frame_at(0.0, 0.0, base + 100),
            frame_at(0.0, 0.0, base + 300),
        ];
        let t: Vec<f64> = time_axis(&frames).into_iter().map(|(t, _)| t).collect();
        assert!(t.windows(2).all(|w| w[1] >= w[0]), "{t:?}");
    }

    #[test]
    fn events_are_deduplicated_and_split() {
        let mut frames = vec![Frame::default(), Frame::default(), Frame::default()];
        frames[0].app.warn = "Strong wind; Low battery".into();
        frames[1].app.warn = "Strong wind".into();
        frames[2].app.tip = "Hello".into();
        let samples: Vec<Sample> = (0..3)
            .map(|i| Sample {
                t: i as f64,
                ..Default::default()
            })
            .collect();
        let events = extract_events(&frames, &samples);
        let msgs: Vec<&str> = events.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(msgs, vec!["Strong wind", "Low battery", "Hello"]);
        assert_eq!(events[0].level, EventLevel::Warning);
        assert_eq!(events[2].level, EventLevel::Info);
    }

    #[test]
    fn maps_synthetic_log_end_to_end() {
        let synth = SynthFlight::demo();
        let log = DJILog::from_bytes(synth.to_bytes()).expect("parse synthetic log");
        assert_eq!(log.version, 6);
        let frames = log.frames(None).expect("frames");
        let flight = build_flight(log.version, &log.details, &frames);

        assert_eq!(flight.samples.len(), synth.samples.len());
        let m = &flight.meta;
        assert_eq!(m.aircraft_name, "Test Mavic");
        assert_eq!(m.product_type, "MavicPro");
        assert!(!m.encrypted);
        assert_eq!(m.distance_m, 240.0); // from log details
        assert_eq!(m.max_height_m, 40.0);
        assert!((m.duration_s - 60.0).abs() < 0.01);
        assert_eq!(
            m.start_time.unwrap().to_rfc3339(),
            "2024-06-01T10:00:00+00:00"
        );
        assert!((m.home_lat.unwrap() - 52.2297).abs() < 1e-6);
        assert!((m.home_lon.unwrap() - 21.0122).abs() < 1e-6);

        // Time axis comes from the log clock (10 Hz).
        assert_eq!(flight.samples[10].t, 1.0);
        assert_eq!(flight.samples[0].timestamp_ms, Some(synth.start_ms));

        // Mid-flight sample: 40 m, flying east at 8 m/s.
        let s = &flight.samples[200];
        assert!((s.height_m - 40.0).abs() < 0.11, "{s:?}");
        assert!((s.h_speed_ms - 8.0).abs() < 0.01);
        assert_eq!(s.v_speed_ms, 0.0);
        assert!(s.is_flying);
        assert_eq!(s.flight_mode, "GPSAtti");
        assert_eq!(s.gps_sats, synth.samples[200].gps_num as i64);
        assert_eq!(s.battery_pct, Some(85.0));
        assert!(s.battery_v.unwrap() > 12.0);
        assert!(s.rc_uplink_pct.is_some() && s.rc_downlink_pct.is_some());

        // Climb phase reports positive vertical speed.
        assert!((flight.samples[100].v_speed_ms - 4.0).abs() < 0.01);

        // Computed track length should agree with the synthetic geometry.
        let pts: Vec<_> = flight
            .samples
            .iter()
            .filter_map(|s| Some((s.t, s.lat?, s.lon?)))
            .collect();
        let d = track_distance(&pts);
        assert!((d - 240.0).abs() < 2.0, "{d}");

        let msgs: Vec<(&str, EventLevel)> = flight
            .events
            .iter()
            .map(|e| (e.message.as_str(), e.level))
            .collect();
        assert!(msgs.contains(&("Take-off", EventLevel::Info)));
        assert!(msgs.contains(&("Landing", EventLevel::Info)));
        assert!(msgs.contains(&("Low battery", EventLevel::Warning)));
        let wind = msgs
            .iter()
            .filter(|(m, _)| *m == "Strong wind. Fly with caution")
            .count();
        assert_eq!(wind, 1, "repeated warning must be de-duplicated");
    }
}
