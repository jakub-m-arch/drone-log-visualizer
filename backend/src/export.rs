//! CSV, GPX and KML export of a stored flight.

use chrono::{DateTime, SecondsFormat, Utc};
use std::fmt::Write;

use crate::db::FlightSummary;
use crate::flight::Sample;

pub fn csv(samples: &[Sample]) -> String {
    let mut out = String::from(
        "time_s,timestamp_utc,latitude,longitude,height_m,altitude_m,h_speed_ms,v_speed_ms,\
         yaw_deg,pitch_deg,roll_deg,battery_pct,battery_v,gps_sats,rc_uplink_pct,\
         rc_downlink_pct,flight_mode,is_flying,gimbal_pitch_deg,is_photo,is_recording\n",
    );
    for s in samples {
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            s.t,
            s.timestamp_ms.and_then(iso_time).unwrap_or_default(),
            opt(s.lat),
            opt(s.lon),
            s.height_m,
            s.altitude_m,
            s.h_speed_ms,
            s.v_speed_ms,
            s.yaw_deg,
            s.pitch_deg,
            s.roll_deg,
            opt(s.battery_pct),
            opt(s.battery_v),
            s.gps_sats,
            opt(s.rc_uplink_pct),
            opt(s.rc_downlink_pct),
            csv_field(&s.flight_mode),
            s.is_flying,
            s.gimbal_pitch_deg,
            s.is_photo,
            s.is_recording,
        );
    }
    out
}

pub fn gpx(flight: &FlightSummary, samples: &[Sample]) -> String {
    let name = xml_escape(&display_name(flight));
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<gpx version=\"1.1\" creator=\"drone-log-visualizer\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n",
    );
    let _ = writeln!(out, "  <metadata><name>{name}</name>");
    if let Some(t) = flight.meta.start_time {
        let _ = writeln!(
            out,
            "    <time>{}</time>",
            t.to_rfc3339_opts(SecondsFormat::Secs, true)
        );
    }
    out.push_str("  </metadata>\n");
    if let (Some(lat), Some(lon)) = (flight.meta.home_lat, flight.meta.home_lon) {
        let _ = writeln!(
            out,
            "  <wpt lat=\"{lat}\" lon=\"{lon}\"><name>Home</name></wpt>"
        );
    }
    let _ = writeln!(out, "  <trk><name>{name}</name><trkseg>");
    for s in samples {
        let (Some(lat), Some(lon)) = (s.lat, s.lon) else {
            continue;
        };
        let _ = write!(
            out,
            "    <trkpt lat=\"{lat}\" lon=\"{lon}\"><ele>{}</ele>",
            s.altitude_m
        );
        if let Some(t) = s.timestamp_ms.and_then(iso_time) {
            let _ = write!(out, "<time>{t}</time>");
        }
        out.push_str("</trkpt>\n");
    }
    out.push_str("  </trkseg></trk>\n</gpx>\n");
    out
}

pub fn kml(flight: &FlightSummary, samples: &[Sample]) -> String {
    let name = xml_escape(&display_name(flight));
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<kml xmlns=\"http://www.opengis.net/kml/2.2\">\n<Document>\n");
    let _ = writeln!(out, "  <name>{name}</name>");
    out.push_str(
        "  <Style id=\"track\"><LineStyle><color>ff0080ff</color><width>3</width></LineStyle></Style>\n",
    );
    if let (Some(lat), Some(lon)) = (flight.meta.home_lat, flight.meta.home_lon) {
        let _ = writeln!(
            out,
            "  <Placemark><name>Home</name><Point><coordinates>{lon},{lat},0</coordinates></Point></Placemark>"
        );
    }
    let _ = writeln!(
        out,
        "  <Placemark><name>{name}</name><styleUrl>#track</styleUrl><LineString>\
         <altitudeMode>absolute</altitudeMode><coordinates>"
    );
    for s in samples {
        if let (Some(lat), Some(lon)) = (s.lat, s.lon) {
            let _ = writeln!(out, "    {lon},{lat},{}", s.altitude_m);
        }
    }
    out.push_str("  </coordinates></LineString></Placemark>\n</Document>\n</kml>\n");
    out
}

/// File name stem for downloads, e.g. `flight-12-2024-06-01`.
pub fn file_stem(flight: &FlightSummary) -> String {
    match flight.meta.start_time {
        Some(t) => format!("flight-{}-{}", flight.id, t.format("%Y-%m-%d")),
        None => format!("flight-{}", flight.id),
    }
}

fn display_name(flight: &FlightSummary) -> String {
    let aircraft = if flight.meta.aircraft_name.is_empty() {
        &flight.meta.product_type
    } else {
        &flight.meta.aircraft_name
    };
    match flight.meta.start_time {
        Some(t) => format!("{aircraft} {}", t.format("%Y-%m-%d %H:%M UTC")),
        None => format!("{aircraft} — {}", flight.file_name),
    }
}

fn iso_time(ms: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn opt(v: Option<f64>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flight::FlightMeta;

    fn flight() -> FlightSummary {
        FlightSummary {
            id: 7,
            file_name: "DJIFlightRecord_x.txt".into(),
            file_size: 1,
            uploaded_at: "2024-06-01T00:00:00Z".into(),
            meta: FlightMeta {
                aircraft_name: "My <Drone> & Co".into(),
                start_time: DateTime::from_timestamp(1_717_236_000, 0),
                home_lat: Some(52.0),
                home_lon: Some(21.0),
                ..Default::default()
            },
        }
    }

    fn samples() -> Vec<Sample> {
        vec![
            Sample {
                t: 0.0,
                timestamp_ms: Some(1_717_236_000_000),
                lat: Some(52.0),
                lon: Some(21.0),
                altitude_m: 100.0,
                flight_mode: "GPS,Atti".into(),
                ..Default::default()
            },
            // No GPS fix: kept in CSV, skipped in tracks.
            Sample {
                t: 0.1,
                ..Default::default()
            },
            Sample {
                t: 0.2,
                timestamp_ms: Some(1_717_236_000_200),
                lat: Some(52.001),
                lon: Some(21.001),
                altitude_m: 110.5,
                ..Default::default()
            },
        ]
    }

    #[test]
    fn csv_has_header_and_one_row_per_sample() {
        let out = csv(&samples());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("time_s,timestamp_utc,latitude"));
        assert!(lines[1].starts_with("0,2024-06-01T10:00:00.000Z,52,21,"));
        assert!(lines[1].contains(",\"GPS,Atti\","), "{}", lines[1]);
        assert!(lines[2].starts_with("0.1,,,,"));
        let cols = lines[0].split(',').count();
        assert_eq!(lines[3].split(',').count(), cols);
    }

    #[test]
    fn gpx_is_escaped_and_skips_points_without_fix() {
        let out = gpx(&flight(), &samples());
        assert!(out.contains("My &lt;Drone&gt; &amp; Co"));
        assert_eq!(out.matches("<trkpt").count(), 2);
        assert!(out.contains(
            "<trkpt lat=\"52.001\" lon=\"21.001\"><ele>110.5</ele><time>2024-06-01T10:00:00.200Z</time></trkpt>"
        ));
        assert!(out.contains("<wpt lat=\"52\" lon=\"21\"><name>Home</name></wpt>"));
    }

    #[test]
    fn kml_uses_lon_lat_alt_order() {
        let out = kml(&flight(), &samples());
        assert!(out.contains("    21.001,52.001,110.5\n"));
        assert!(out.contains("<coordinates>21,52,0</coordinates>"));
        assert!(!out.contains("<Drone>"));
    }

    #[test]
    fn file_stem_contains_date() {
        assert_eq!(file_stem(&flight()), "flight-7-2024-06-01");
    }
}
