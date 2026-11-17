//! The pacemaker bench: Otto's `Pacemaker` on a virtual display with a
//! perfect VBlank, deterministic, no GPU or DRM device needed.
//!
//! ```sh
//! cargo bench --bench pacemaker                       # Markdown table
//! cargo bench --bench pacemaker -- --json             # the reports as JSON
//! cargo bench --bench pacemaker -- --edid /sys/class/drm/card1-eDP-1/edid
//! cargo bench --bench pacemaker -- --baseline benches/pacemaker.baseline.json
//! cargo bench --bench pacemaker -- --write-baseline benches/pacemaker.baseline.json
//! ```
//!
//! `--baseline` exits non-zero when a scenario got worse than the committed
//! reports (see `pacemaker::sim::regressions`); `--write-baseline`
//! refreshes them after a change that improves pacing. `--scenario NAME`
//! runs one. The scenarios and what they model: `src/pacemaker/sim.rs`.

use std::process::ExitCode;

use otto::pacemaker::{edid, sim};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut json = false;
    let mut edid_path = None;
    let mut baseline = None;
    let mut write_baseline = None;
    let mut only = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--edid" => edid_path = args.next(),
            "--baseline" => baseline = args.next(),
            "--write-baseline" => write_baseline = args.next(),
            "--scenario" => only = args.next(),
            // `cargo bench` passes `--bench`; anything else is a filter cargo
            // hands every bench target.
            _ => {}
        }
    }

    let scenarios = match &edid_path {
        Some(path) => {
            let display = match std::fs::read(path)
                .map_err(|e| e.to_string())
                .and_then(|blob| edid::parse(&blob).map_err(|e| e.to_string()))
            {
                Ok(display) => display,
                Err(err) => {
                    eprintln!("{path}: {err}");
                    return ExitCode::FAILURE;
                }
            };
            eprintln!("display from {path}: {} {}", display.name, display.label());
            sim::standard_on(&display)
        }
        None => sim::standard(),
    };
    let reports: Vec<sim::Report> = scenarios
        .iter()
        .filter(|s| only.as_deref().is_none_or(|name| s.name == name))
        .map(sim::Scenario::run)
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&reports).unwrap());
    } else {
        print!("{}", sim::markdown(&reports));
    }

    if let Some(path) = write_baseline {
        let text = serde_json::to_string_pretty(&reports).unwrap() + "\n";
        if let Err(err) = std::fs::write(&path, text) {
            eprintln!("{path}: {err}");
            return ExitCode::FAILURE;
        }
        eprintln!("wrote {path}");
    }

    if let Some(path) = baseline {
        let base: Vec<sim::Report> = match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
        {
            Ok(base) => base,
            Err(err) => {
                eprintln!("{path}: {err}");
                return ExitCode::FAILURE;
            }
        };
        let found = sim::regressions(&base, &reports);
        if !found.is_empty() {
            println!("\n**Frame pacing got worse than `{path}`:**\n");
            for line in &found {
                println!("- {line}");
            }
            println!(
                "\nIf that is intended, refresh the baseline: \
                 `cargo bench --bench pacemaker -- --write-baseline {path}`"
            );
            return ExitCode::FAILURE;
        }
        // The other way round, a regression of the baseline is an
        // improvement of this run.
        let better = sim::regressions(&reports, &base);
        if !better.is_empty() {
            println!("\nBetter than `{path}`:\n");
            for line in &better {
                println!("- {line}");
            }
            println!("\nRefresh it with `--write-baseline {path}` to lock the change in.");
        }
    }
    ExitCode::SUCCESS
}
