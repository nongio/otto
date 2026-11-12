//! Stress for lock-order deadlocks between the compositor thread and the
//! threads that touch the scene behind its back.
//!
//! Several things move layers off the main loop: the genie runs on a tokio
//! task, the context menu and the app switcher update their views from tokio
//! tasks, and the dock's model feeds wake on timers. Meanwhile the compositor
//! thread draws, and drawing reads the scene — a window preview in exposé reads
//! it again from inside a draw. A lock taken in the wrong order on either side
//! freezes the whole session.
//!
//! For a while (`OTTO_STRESS_SECS`, 8 s by default) this maps and closes
//! windows, commits to them, minimises and restores them, opens and closes
//! exposé and the app switcher, and draws composite frames, while threads of
//! its own mutate, re-parent and read layers of the same scene. A watchdog
//! fails the test when any of them stops making progress, and saves every
//! thread's stack with gdb first (when gdb is installed), so a hang reads as
//! a stack trace rather than a CI timeout.
//!
//! The run is seeded (`OTTO_STRESS_SEED`, printed at the start) so a hang can
//! be replayed; `OTTO_STRESS_MUTATORS=0` and `OTTO_STRESS_READER=0` leave out
//! the test's own threads, to see which of them a hang needs.
//!
//! ```sh
//! OTTO_STRESS_SECS=120 cargo test --features headless --test deadlock_stress -- --nocapture
//! ```

#[cfg(feature = "headless")]
mod deadlock_stress_tests {
    use layers::prelude::{Layer, Transition};
    use layers::skia;
    use layers::types::Color;
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::{TestClient, TestToplevel};
    use serial_test::serial;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// How long a worker may go without a step before the test calls it a hang.
    const STALL: Duration = Duration::from_secs(6);
    const APPS: [&str; 4] = [
        "org.gnome.Nautilus",
        "org.mozilla.firefox",
        "com.mitchellh.ghostty",
        "stress.unknown",
    ];

    thread_local! {
        /// The composite framebuffer and the commit it was drawn at, kept on
        /// the compositor thread between frames.
        static FRAME: RefCell<Option<(
            skia::Surface,
            Option<smithay::backend::renderer::utils::CommitCounter>,
        )>> = const { RefCell::new(None) };
    }

    /// A small deterministic generator: a failing run replays with the same
    /// `OTTO_STRESS_SEED`.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn unit(&mut self) -> f32 {
            (self.next() % 1000) as f32 / 1000.0
        }
    }

    /// Steps taken by one worker, watched for stalls.
    #[derive(Clone)]
    struct Progress {
        name: &'static str,
        steps: Arc<AtomicU64>,
    }
    impl Progress {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                steps: Arc::new(AtomicU64::new(0)),
            }
        }
        fn step(&self) {
            self.steps.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Fails the process when a worker stops stepping, after saving every
    /// thread's stack.
    fn watchdog(workers: Vec<Progress>, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name("stress-watchdog".into())
            .spawn(move || {
                let mut last: Vec<(u64, Instant)> =
                    workers.iter().map(|_| (0, Instant::now())).collect();
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(250));
                    for (worker, (steps, since)) in workers.iter().zip(last.iter_mut()) {
                        let now = worker.steps.load(Ordering::Relaxed);
                        if now != *steps {
                            *steps = now;
                            *since = Instant::now();
                        } else if since.elapsed() >= STALL && !stop.load(Ordering::Relaxed) {
                            hang(worker.name, now);
                        }
                    }
                }
            })
            .unwrap()
    }

    fn hang(worker: &str, steps: u64) -> ! {
        eprintln!("\nDEADLOCK? `{worker}` made no progress for {STALL:?} after {steps} steps");
        let pid = std::process::id().to_string();
        // Yama lets a process be traced only by its ancestors; let gdb, our
        // child, in.
        unsafe {
            libc::prctl(libc::PR_SET_PTRACER, -1i64 as libc::c_ulong, 0, 0, 0);
        }
        let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("deadlock-stress-{pid}.txt"));
        // Straight into the file: gdb stops every thread of this process,
        // this one too, so nothing here could drain a pipe while it writes.
        let file = std::fs::File::create(&out).expect("the stacks file");
        let ran = std::process::Command::new("gdb")
            .args(["-p", &pid, "-batch", "-nx"])
            .args(["-iex", "set debuginfod enabled off"])
            .args(["-ex", "set pagination off", "-ex", "thread apply all bt 50"])
            .stdin(std::process::Stdio::null())
            .stdout(file)
            .stderr(std::process::Stdio::null())
            .status();
        match ran {
            Ok(_) => {
                let stacks = std::fs::read_to_string(&out).unwrap_or_default();
                eprintln!("{}", summarise(&stacks));
                eprintln!("every thread's stack: {}", out.display());
            }
            Err(err) => eprintln!("no stacks: gdb did not run ({err})"),
        }
        std::process::exit(1);
    }

    /// The threads blocked on a lock, each with its frames from Otto, lay-rs
    /// and this test: enough to read who holds what and who waits for it.
    fn summarise(stacks: &str) -> String {
        let mut out = String::new();
        for thread in stacks.split("\nThread ").skip(1) {
            let mut lines = thread.lines();
            let header = lines.next().unwrap_or_default();
            let frames: Vec<&str> = lines.filter(|l| l.starts_with('#')).collect();
            if !frames
                .iter()
                .any(|f| f.contains("futex") || f.contains("RwLock"))
            {
                continue;
            }
            out.push_str("\nThread ");
            out.push_str(header);
            out.push('\n');
            for frame in frames {
                let call = match frame.split_once(" in ") {
                    Some((_, call)) => call,
                    // An inlined frame has no address: `#4  std::…`.
                    None => frame.split_once("  ").map_or(frame, |(_, call)| call),
                };
                let call = call.split(" (").next().unwrap_or(call);
                let ours = ["otto::", "layers::", "deadlock_stress::"]
                    .iter()
                    .any(|c| call.starts_with(c));
                if ours || call.contains("::read<") || call.contains("::write<") {
                    let at = frame
                        .rsplit(" at ")
                        .next()
                        .filter(|_| frame.contains(" at "));
                    out.push_str(&format!("    {call}  {}\n", at.unwrap_or("")));
                }
            }
        }
        out
    }

    /// One frame through `SceneElement`, the way the udev composite path
    /// draws: into a framebuffer kept between frames, repainting what the
    /// element reports damaged.
    fn draw_frame(handle: &HeadlessHandle) {
        use smithay::backend::renderer::element::Element;
        handle.with_state(|state| {
            let output = state.workspaces.outputs().next().unwrap().clone();
            let size = output.current_mode().unwrap().size;
            if state.scene_element.size == (0.0, 0.0) {
                state.scene_element.set_size(size.w as f32, size.h as f32);
            }
            state.scene_element.update();
            let ows = state.workspaces.output_workspaces.values().next().unwrap();
            let element = state.scene_element.for_output_layer(&ows.output_layer);
            let scale = smithay::utils::Scale::from(output.current_scale().fractional_scale());
            FRAME.with(|frame| {
                let mut frame = frame.borrow_mut();
                let (surface, last) = frame.get_or_insert_with(|| {
                    (
                        skia::surfaces::raster_n32_premul((size.w, size.h)).unwrap(),
                        None,
                    )
                });
                let damage = element.damage_since(scale, *last);
                *last = Some(element.current_commit());
                let dst = smithay::utils::Rectangle::new((0, 0).into(), (size.w, size.h).into());
                element.draw_scene(surface.canvas(), dst, &damage);
            });
        });
    }

    /// Mutates a subtree of the scene from its own thread, the way the genie
    /// and the views updated from tokio tasks do: layers added and removed,
    /// animated, frosted, measured.
    fn mutator(
        engine: Arc<layers::prelude::Engine>,
        parent: Layer,
        seed: u64,
        progress: Progress,
        stop: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name(format!("stress-{}", progress.name))
            .spawn(move || {
                let mut rng = Rng(seed);
                // Containers stay put under `parent`; leaves come, go and move
                // between them. Only a leaf is ever moved, so no layer lands
                // under its own descendant.
                let containers: Vec<Layer> = (0..4)
                    .map(|_| {
                        let layer = engine.new_layer();
                        layer.set_layout_style(layers::taffy::Style {
                            position: layers::taffy::Position::Absolute,
                            ..Default::default()
                        });
                        let _ = parent.add_sublayer(&layer);
                        layer
                    })
                    .collect();
                let mut layers: Vec<Layer> = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    match rng.below(10) {
                        0..=2 if layers.len() < 24 => {
                            let layer = engine.new_layer();
                            layer.set_layout_style(layers::taffy::Style {
                                position: layers::taffy::Position::Absolute,
                                ..Default::default()
                            });
                            layer.set_size(
                                layers::types::Size::points(
                                    20.0 + 200.0 * rng.unit(),
                                    20.0 + 150.0 * rng.unit(),
                                ),
                                None,
                            );
                            layer.set_background_color(
                                Color::new_rgba(rng.unit(), rng.unit(), rng.unit(), 0.6),
                                None,
                            );
                            if rng.below(3) == 0 {
                                layer.set_blend_mode(layers::types::BlendMode::BackgroundBlur);
                            }
                            let _ = containers[rng.below(containers.len())].add_sublayer(&layer);
                            layers.push(layer);
                        }
                        3 if !layers.is_empty() => {
                            let layer = layers.swap_remove(rng.below(layers.len()));
                            layer.remove();
                        }
                        4 if !layers.is_empty() => {
                            // Re-parent: what the genie does with a window.
                            let layer = &layers[rng.below(layers.len())];
                            let _ = containers[rng.below(containers.len())].add_sublayer(layer);
                        }
                        5..=7 if !layers.is_empty() => {
                            let layer = &layers[rng.below(layers.len())];
                            let transition = (rng.below(2) == 0)
                                .then(|| Transition::ease_out(0.05 + 0.3 * rng.unit()));
                            layer.set_position(
                                (900.0 * rng.unit(), 560.0 * rng.unit()),
                                transition.clone(),
                            );
                            layer.set_opacity(rng.unit(), transition);
                        }
                        _ => {
                            if let Some(layer) = layers.get(rng.below(layers.len().max(1))) {
                                let _ = layer.render_bounds_transformed();
                                let _ = layer.render_bounds_with_children();
                            }
                            engine.scene().with_arena(|_| {
                                // Nested, like a mirror drawn inside a draw.
                                engine.scene().with_arena(|arena| arena.len())
                            });
                        }
                    }
                    progress.step();
                    // Paced, or the writers starve everyone else of the lock
                    // and the compositor gets a turn a second.
                    std::thread::sleep(Duration::from_millis(1));
                }
                for layer in layers.into_iter().chain(containers) {
                    layer.remove();
                }
            })
            .unwrap()
    }

    /// Draws the whole output tree from scratch on its own thread, the way the
    /// titlebar test's reference render does: a reader that holds the scene
    /// for a whole draw while the other threads write.
    fn reader(
        engine: Arc<layers::prelude::Engine>,
        root: layers::prelude::NodeRef,
        progress: Progress,
        stop: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name("stress-reader".into())
            .spawn(move || {
                let mut surface = skia::surfaces::raster_n32_premul((480, 270)).unwrap();
                while !stop.load(Ordering::Relaxed) {
                    let scene = engine.scene().clone();
                    scene.with_arena(|arena| {
                        scene.with_renderable_arena(|renderables| {
                            layers::drawing::render_node_tree(
                                root,
                                arena,
                                renderables,
                                surface.canvas(),
                                0.25,
                                None,
                                None,
                                None,
                            );
                        })
                    });
                    progress.step();
                    // Spaced out: a whole draw holds the scene, and back to back
                    // they would leave the compositor no turn to write.
                    std::thread::sleep(Duration::from_millis(25));
                }
            })
            .unwrap()
    }

    struct Window {
        title: String,
        toplevel: Arc<Mutex<TestToplevel>>,
        minimised: bool,
    }

    fn close(window: Window) {
        let top = window.toplevel.lock().unwrap();
        top.toplevel.as_ref().unwrap().destroy();
        top.xdg_surface.as_ref().unwrap().destroy();
        top.surface.destroy();
    }

    #[test]
    #[serial]
    fn the_compositor_keeps_turning_under_concurrent_scene_access() {
        let secs: u64 = std::env::var("OTTO_STRESS_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8);
        let seed: u64 = std::env::var("OTTO_STRESS_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0x5eed_0770);
        eprintln!("deadlock stress: {secs} s, OTTO_STRESS_SEED={seed}");

        // Small, so the CPU raster frames are cheap and the compositor turns
        // often; a fractional scale, as the sessions that froze ran at.
        let handle = HeadlessHandle::start(HeadlessConfig {
            width: 960,
            height: 600,
            scale: Some(1.25),
        });
        handle.settle(200);

        let stop = Arc::new(AtomicBool::new(false));
        let main = Progress::new("compositor");
        let workers = [Progress::new("mutator-a"), Progress::new("mutator-b")];
        let read = Progress::new("reader");

        // The threads' own subtrees hang under the output layer, so they are
        // drawn, frosted and mirrored along with the windows.
        let (engine, parents, root) = handle.query(|state| {
            let ows = state.workspaces.output_workspaces.values().next().unwrap();
            let output_layer = ows.output_layer.clone();
            let parents: Vec<Layer> = (0..2)
                .map(|_| {
                    let layer = state.layers_engine.new_layer();
                    layer.set_layout_style(layers::taffy::Style {
                        position: layers::taffy::Position::Absolute,
                        ..Default::default()
                    });
                    let _ = output_layer.add_sublayer(&layer);
                    layer
                })
                .collect();
            (state.layers_engine.clone(), parents, output_layer.id())
        });

        let knob = |name: &str| std::env::var(name).map(|v| v != "0").unwrap_or(true);
        let mut threads: Vec<_> = workers
            .iter()
            .zip(parents)
            .filter(|_| knob("OTTO_STRESS_MUTATORS"))
            .enumerate()
            .map(|(i, (progress, parent))| {
                mutator(
                    engine.clone(),
                    parent,
                    seed.wrapping_add(i as u64 + 1) | 1,
                    progress.clone(),
                    stop.clone(),
                )
            })
            .collect();
        let mut watched = vec![main.clone()];
        if knob("OTTO_STRESS_READER") {
            threads.push(reader(engine.clone(), root, read.clone(), stop.clone()));
            watched.push(read);
        }
        if knob("OTTO_STRESS_MUTATORS") {
            watched.extend(workers.iter().cloned());
        }
        let dog = watchdog(watched, stop.clone());

        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        let mut rng = Rng(seed | 1);
        let mut windows: Vec<Window> = Vec::new();
        let mut serial = 0usize;
        let mut steps = 0u64;
        let end = Instant::now() + Duration::from_secs(secs);

        while Instant::now() < end {
            match rng.below(16) {
                0 | 1 if windows.len() < 8 => {
                    serial += 1;
                    let title = format!("stress-{serial}");
                    let app = APPS[rng.below(APPS.len())];
                    let toplevel = client.create_toplevel_with_app_id(
                        &title,
                        app,
                        120 + rng.below(400) as u32,
                        90 + rng.below(300) as u32,
                    );
                    windows.push(Window {
                        title,
                        toplevel,
                        minimised: false,
                    });
                }
                2 if windows.len() > 1 => close(windows.swap_remove(rng.below(windows.len()))),
                3 | 4 if !windows.is_empty() => {
                    let i = rng.below(windows.len());
                    let window = &mut windows[i];
                    if window.minimised {
                        handle.unminimize_window(&window.title);
                    } else {
                        handle.minimize_window(&window.title);
                    }
                    window.minimised = !window.minimised;
                }
                5 => handle.toggle_expose(),
                6 => {
                    handle.app_switcher_next();
                    if rng.below(2) == 0 {
                        handle.app_switcher_commit();
                    }
                }
                7 if !windows.is_empty() => {
                    let window = &windows[rng.below(windows.len())];
                    handle.focus_window(&window.title);
                }
                8 if !windows.is_empty() => {
                    let window = &windows[rng.below(windows.len())];
                    handle.move_window(
                        &window.title,
                        rng.below(700) as i32 - 100,
                        rng.below(400) as i32,
                    );
                }
                _ => {
                    for window in &windows {
                        window.toplevel.lock().unwrap().commit_frame();
                    }
                }
            }
            let _ = client.roundtrip_timeout(STALL);
            main.step();
            handle.tick(1.0 / 60.0);
            draw_frame(&handle);
            main.step();
            steps += 1;
        }

        if handle.is_expose_active() {
            handle.toggle_expose();
        }
        handle.app_switcher_commit();
        stop.store(true, Ordering::Relaxed);
        for thread in threads {
            thread.join().expect("a stress thread panicked");
        }
        dog.join().unwrap();
        for window in windows.drain(..) {
            close(window);
        }
        let _ = client.roundtrip();
        handle.settle(200);
        draw_frame(&handle);
        eprintln!("deadlock stress: {steps} compositor steps");
        assert!(handle.is_running());
        handle.stop();
    }
}
