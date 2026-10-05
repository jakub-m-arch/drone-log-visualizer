//! Writes a synthetic DJI flight log for trying out the viewer without a real
//! flight record:
//!
//!     cargo run --bin gen-sample-log -- DJIFlightRecord_sample.txt
//!
//! With `--encrypted` it writes a format v14 log encrypted with made-up keys.
//! The real DJI API cannot decrypt it, so it is only useful for checking the
//! error messages around `DJI_API_KEY`.
use dji_log_viewer::synthetic::{SynthFlight, SynthKeys};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let encrypted = args.iter().any(|a| a == "--encrypted");
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "DJIFlightRecord_sample.txt".into());
    let flight = SynthFlight::demo();
    let bytes = if encrypted {
        flight.to_encrypted_bytes(&SynthKeys::new(1))
    } else {
        flight.to_bytes()
    };
    std::fs::write(&path, bytes).expect("write sample log");
    println!("wrote {path}");
}
