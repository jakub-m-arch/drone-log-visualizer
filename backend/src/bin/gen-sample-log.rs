//! Writes a synthetic DJI flight log for trying out the viewer without a real
//! flight record:
//!
//!     cargo run --bin gen-sample-log -- DJIFlightRecord_sample.txt
//!
//! With `--showcase <dir>` it writes the three longer, scenic flights over
//! Kazimierz Dolny from `samples/` (and the README screenshots) into `<dir>`.
//!
//! With `--encrypted` it writes a format v14 log encrypted with made-up keys.
//! The real DJI API cannot decrypt it, so it is only useful for checking the
//! error messages around `DJI_API_KEY`.
use dji_log_viewer::synthetic::{SHOWCASE_FILES, SynthFlight, SynthKeys};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let encrypted = args.iter().any(|a| a == "--encrypted");
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "DJIFlightRecord_sample.txt".into());
    if args.iter().any(|a| a == "--showcase") {
        let dir = std::path::Path::new(
            args.iter()
                .find(|a| !a.starts_with("--"))
                .map_or(".", |s| s),
        );
        for (v, name) in SHOWCASE_FILES.iter().enumerate() {
            let file = dir.join(name);
            std::fs::write(&file, SynthFlight::showcase(v as u8).to_bytes())
                .expect("write sample log");
            println!("wrote {}", file.display());
        }
        return;
    }
    let flight = SynthFlight::demo();
    let bytes = if encrypted {
        flight.to_encrypted_bytes(&SynthKeys::new(1))
    } else {
        flight.to_bytes()
    };
    std::fs::write(&path, bytes).expect("write sample log");
    println!("wrote {path}");
}
