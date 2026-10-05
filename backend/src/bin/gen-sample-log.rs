//! Writes a synthetic (unencrypted, format v6) DJI flight log for trying out
//! the UI without a real flight record:
//!
//!     cargo run --bin gen-sample-log -- DJIFlightRecord_sample.txt
use dji_log_viewer::synthetic::SynthFlight;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "DJIFlightRecord_sample.txt".into());
    std::fs::write(&path, SynthFlight::demo().to_bytes()).expect("write sample log");
    println!("wrote {path}");
}
