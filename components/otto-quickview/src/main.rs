//! otto-quickview — the previewer's binary.
//!
//! The preview itself is a *library* the file views embed
//! ([`otto_quickview`]); this binary exists for the cases that have no host:
//!
//! * `--decode-worker` — the sandboxed decoder, re-executed per preview with
//!   one file on descriptor 3 and a payload on stdout. Never run by hand, and
//!   the reason every embedding host must call
//!   [`otto_quickview::run_worker_if_requested`] first thing in `main`;
//! * `--describe` — decode one file and print what the previewer made of it,
//!   which exercises the decoders with no display attached;
//! * `--render OUT.png` — draw a preview through the same
//!   `otto_kit::preview::draw` a host uses, so a PNG here is what a host will
//!   show. This is the integration check between the previewer and its hosts;
//! * `--filmstrip OUT.png` — the entrance, sampled.
//!
//! Why the preview is embedded rather than a service of its own is in
//! `specs/quickview.md`.

mod render;

use std::path::PathBuf;

use otto_quickview::decode::Request;
use otto_quickview::payload::PreviewPayload;
use otto_quickview::{opening, sandbox, spawn};

#[tokio::main]
async fn main() {
    // The worker must not initialise logging to a shared sink or inherit the
    // parent's subscriber; it writes a payload on stdout and nothing else.
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    // The self-test runs in a child with a piped stdout, exactly as a worker
    // does. It cannot run in this process: `RLIMIT_FSIZE = 0` means the very
    // first write to a *file-backed* stdout raises SIGXFSZ, so running it with
    // output redirected would kill the reporter before it reported. The worker
    // is unaffected because the parent always hands it a pipe — but the hazard
    // is real enough to be worth reproducing here rather than designing around.
    if arguments.first().map(String::as_str) == Some("--sandbox-selftest") {
        match std::process::Command::new(std::env::current_exe().expect("own path"))
            .arg("--sandbox-selftest-child")
            .stdout(std::process::Stdio::piped())
            .output()
        {
            Ok(output) => {
                print!("{}", String::from_utf8_lossy(&output.stdout));
                if !output.status.success() {
                    eprintln!("self-test child exited with {}", output.status);
                }
            }
            Err(err) => eprintln!("could not run the self-test: {err}"),
        }
        return;
    }
    if arguments.first().map(String::as_str) == Some("--sandbox-selftest-child") {
        // SAFETY: nothing else is running in this process yet, which is the
        // same condition the worker applies it under.
        if let Err(err) = unsafe { sandbox::apply(sandbox::Budget::default()) } {
            eprintln!("could not apply the sandbox: {err}");
            std::process::exit(1);
        }
        let result = sandbox::self_test();
        println!("address space capped     {}", result.address_space_capped);
        println!("cannot grow a file       {}", result.cannot_grow_a_file);
        println!("network unreachable      {}", result.network_unreachable);
        println!(
            "can open other files     {}  {}",
            result.can_still_open_other_files,
            if result.can_still_open_other_files {
                "← not contained; see sandbox.rs"
            } else {
                ""
            }
        );
        return;
    }
    // The same entry point every embedding host calls, so the worker path is
    // identical whether it was re-executed from here or from a file browser.
    otto_quickview::run_worker_if_requested();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Before the first string is looked up and before anything is drawn.
    otto_kit::i18n::init_from_desktop();

    // The icon theme comes from the XDG settings portal, and the watcher that
    // reads it needs a tokio runtime present — which is why `main` is async.
    // Without it `current_icon_theme()` stays empty, `freedesktop-icons`
    // searches `hicolor` alone, and since hicolor ships no mimetype icons a
    // listing draws with no icons at all. That looks like an icon bug and is
    // really a runtime one.
    otto_kit::icon_theme::spawn_icon_theme_watcher();

    let mut paths: Vec<PathBuf> = Vec::new();
    let mut describe = false;
    let mut png_out: Option<String> = None;
    let mut filmstrip: Option<String> = None;
    let mut dark = false;
    let mut request = Request::default();
    let mut index = 0usize;

    let mut rest = arguments.iter().peekable();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            // What the .service file passes. It means "no paths": be the
            // service and wait for an Open.
            "--serve" => {}
            "--describe" => describe = true,
            "--render" => png_out = rest.next().cloned(),
            "--filmstrip" => filmstrip = rest.next().cloned(),
            "--dark" => dark = true,
            "--page" => {
                request.page = rest.next().and_then(|v| v.parse().ok()).unwrap_or(1);
            }
            "--zoom" => {
                request.zoom = rest.next().and_then(|v| v.parse().ok()).unwrap_or(1.0);
            }
            "--width" => {
                request.width = rest.next().and_then(|v| v.parse().ok()).unwrap_or(1600);
            }
            "--height" => {
                request.height = rest.next().and_then(|v| v.parse().ok()).unwrap_or(1200);
            }
            "--index" => {
                index = rest.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            "--help" | "-h" => {
                println!(
                    "usage: otto-quickview [--describe] [--render OUT.png] [--dark] [--index N] [--page N] [--zoom F] [--width W] [--height H] PATH..."
                );
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("otto-quickview: unknown option {other}");
                std::process::exit(2);
            }
            other => paths.push(PathBuf::from(other)),
        }
    }

    if paths.is_empty() {
        eprintln!("otto-quickview: nothing to preview");
        eprintln!(
            "usage: otto-quickview [--describe|--render OUT.png|--filmstrip OUT.png] PATH..."
        );
        std::process::exit(2);
    }
    let selected = paths
        .get(index)
        .cloned()
        .unwrap_or_else(|| paths[0].clone());

    if describe {
        let payload = spawn::decode_path(&selected, &request);
        print_payload(&selected, &payload);
        return;
    }

    let payload = spawn::decode_path(&selected, &request);

    if let Some(out) = filmstrip {
        let title = selected
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let frames = [0.0, 0.08, 0.18, 0.32, 0.55, 0.78, 0.9, 1.0];
        match render::opening_filmstrip(&payload, &title, &frames, dark) {
            Some(png) => {
                let _ = std::fs::write(&out, png);
                println!("{out}");
            }
            None => eprintln!("otto-quickview: could not render the filmstrip"),
        }
        return;
    }

    // `--render` draws through the same `otto_kit::preview::draw` the window
    // will use, so a PNG here is what the window will show.
    if let Some(out) = png_out {
        let title = selected
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        match render::to_png(
            &payload,
            &title,
            request.width as i32,
            request.height as i32,
            dark,
        ) {
            Some(png) => match std::fs::write(&out, png) {
                Ok(()) => println!("{out}"),
                Err(err) => {
                    eprintln!("otto-quickview: cannot write {out}: {err}");
                    std::process::exit(1);
                }
            },
            None => {
                eprintln!("otto-quickview: could not render the preview");
                std::process::exit(1);
            }
        }
        return;
    }

    print_payload(&selected, &payload);
}

/// Render a payload as text. This is what `--describe` prints, and it is the
/// fastest way to see what a decoder actually produced.
fn print_payload(path: &std::path::Path, payload: &PreviewPayload) {
    println!("{}", path.display());
    match payload {
        PreviewPayload::Pixels {
            pixels,
            pages,
            page,
        } => {
            println!(
                "  pixels    {}×{} (source {}×{}, native scale {:.2}×)",
                pixels.width,
                pixels.height,
                pixels.intrinsic_width,
                pixels.intrinsic_height,
                pixels.native_scale()
            );
            if *pages > 1 {
                println!("  page      {page} of {pages}");
            }
            println!("  buffer    {} bytes", pixels.data.len());
        }
        PreviewPayload::Text {
            lines,
            truncated,
            language,
        } => {
            println!(
                "  text      {} lines{}{}",
                lines.len(),
                if *truncated { ", truncated" } else { "" },
                if language.is_empty() {
                    String::new()
                } else {
                    format!(", {language}")
                }
            );
            for line in lines.iter().take(5) {
                println!("    │ {line}");
            }
            if lines.len() > 5 {
                println!("    │ …");
            }
        }
        PreviewPayload::Rows {
            rows,
            truncated,
            summary,
        } => {
            println!(
                "  listing   {summary}{}",
                if *truncated { " (truncated)" } else { "" }
            );
            for row in rows.iter().take(8) {
                println!(
                    "    {} {}{}",
                    if row.is_dir { "📁" } else { "  " },
                    row.name,
                    if row.size > 0 {
                        format!("  ({})", otto_kit::preview::human_size(row.size))
                    } else {
                        String::new()
                    }
                );
            }
            if rows.len() > 8 {
                println!("    … {} more", rows.len() - 8);
            }
        }
        PreviewPayload::Card {
            title,
            subtitle,
            facts,
            hero,
            icon,
        } => {
            println!("  card      {title}");
            println!("            {subtitle}");
            for fact in facts {
                println!("    {:<12} {}", fact.key, fact.value);
            }
            if let Some(hero) = hero {
                println!("    (artwork  {}×{})", hero.width, hero.height);
            } else if let Some(first) = icon.first() {
                println!("    (icon     {first})");
            }
        }
        PreviewPayload::Unavailable { reason, icon } => {
            println!("  no preview: {reason}");
            if let Some(first) = icon.first() {
                println!("    (icon     {first})");
            }
        }
    }
}
