//! Render every panel state to PNGs.
//!
//! The panel normally lives on a fullscreen overlay surface with exclusive
//! keyboard input, which is a hostile thing to launch while working on it.
//! This draws the same code into an offscreen raster surface instead:
//!
//! ```sh
//! cargo run -p otto-auth-ui --example preview -- /tmp/panel
//! ```
//!
//! Pass a wallpaper to see it frosted rather than the fallback gradient:
//!
//! ```sh
//! OTTO_PANEL_WALLPAPER=resources/background.jpg \
//!     cargo run -p otto-auth-ui --example preview
//! ```

use std::path::{Path, PathBuf};

use layers::prelude::Engine;
use otto_auth_ui::{Appearance, Field, Finger, Panel, Status, User, View};

/// A laptop panel's logical size — 2880×1920 at scale 2.
const WIDTH: i32 = 1440;
const HEIGHT: i32 = 960;

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/otto-panel-preview"));
    std::fs::create_dir_all(&out_dir).expect("cannot create the output directory");

    let mut appearance = Appearance::load();
    if let Some(wallpaper) = std::env::var_os("OTTO_PANEL_WALLPAPER") {
        appearance.wallpaper = Some(PathBuf::from(wallpaper));
    }
    // The panel is a scene, so the preview needs an engine — but not a
    // compositor: `draw_scene` renders the same tree into any canvas.
    let engine = Engine::create(WIDTH as f32, HEIGHT as f32);
    let mut panel = Panel::new(appearance, engine.clone(), None);
    panel.set_size(WIDTH as f32, HEIGHT as f32);

    // A real account if this machine has one, so the avatar path is exercised;
    // otherwise a made-up user, which shows the initials fallback.
    let user = User::current().unwrap_or_else(|| User {
        name: "riccardo".into(),
        display_name: "Riccardo Canalicchio".into(),
        avatar: None,
    });

    let states: Vec<(&str, View)> = vec![
        (
            "username",
            View {
                user: None,
                prompt: "Username",
                field: Field::Text("ricc"),
                status: None,
                session: Some("Otto (current build)"),
                busy: None,
                power: true,
            },
        ),
        (
            "password",
            View {
                user: Some(&user),
                prompt: "Password",
                field: Field::Secret(6),
                status: None,
                session: Some("Otto (current build)"),
                busy: None,
                power: true,
            },
        ),
        (
            "fingerprint",
            View {
                user: Some(&user),
                prompt: "Password",
                field: Field::Secret(0),
                status: Some(Status::Fingerprint(
                    "Place your finger on the reader",
                    Finger::Awaited,
                )),
                session: Some("Otto (current build)"),
                busy: None,
                power: true,
            },
        ),
        (
            "accepted",
            View {
                user: Some(&user),
                prompt: "Password",
                field: Field::Secret(0),
                status: Some(Status::Fingerprint("Authenticated", Finger::Accepted)),
                session: Some("Otto (current build)"),
                busy: None,
                power: false,
            },
        ),
        (
            "error",
            View {
                user: Some(&user),
                prompt: "Password",
                field: Field::Secret(0),
                status: Some(Status::Error("Authentication failed")),
                session: Some("Otto (current build)"),
                busy: None,
                power: true,
            },
        ),
        (
            "starting",
            View {
                user: Some(&user),
                prompt: "Password",
                field: Field::Secret(0),
                status: None,
                session: Some("Otto (current build)"),
                busy: Some("Starting session…"),
                power: false,
            },
        ),
        // What a lock screen shows: no session picker, and the subject is
        // fixed because there is nobody else it could be.
        (
            "lock",
            View {
                user: Some(&user),
                prompt: "Enter your password to unlock",
                field: Field::Secret(3),
                status: None,
                session: None,
                busy: None,
                power: true,
            },
        ),
    ];

    for (name, view) in &states {
        let path = out_dir.join(format!("{name}.png"));
        render(&mut panel, &engine, view, &path);
        println!("{}", path.display());
    }
}

fn render(panel: &mut Panel, engine: &std::sync::Arc<Engine>, view: &View, path: &Path) {
    let mut surface = skia_safe::surfaces::raster_n32_premul((WIDTH, HEIGHT))
        .expect("cannot create the raster surface");

    panel.update(view);
    // Settle the transitions: a still frame wants the state they animate to,
    // not the first step towards it.
    for _ in 0..60 {
        engine.update(0.016);
    }
    // The Touch ID mark reads the wall clock, so engine ticks alone leave it at
    // its first frame. Spend real time on it: enough for an accepted mark to
    // finish, while a looping one is caught wherever it happens to be.
    if panel.wants_frames() {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(900);
        while std::time::Instant::now() < deadline {
            panel.animate();
            engine.update(0.016);
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
    }
    layers::prelude::draw_scene(surface.canvas(), engine.scene(), panel.layer().id());

    let image = surface.image_snapshot();
    let data = image
        .encode(None, skia_safe::EncodedImageFormat::PNG, 100)
        .expect("cannot encode the preview");
    std::fs::write(path, data.as_bytes()).expect("cannot write the preview");
}
