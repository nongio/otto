//! A window restored while it is still being minimised comes back whole.
//!
//! Minimising plays the genie on a task of its own and, once it is done,
//! moves the window into its dock drawer at mini scale. A restore that lands
//! before the genie has finished must win: the window is back on its
//! workspace, full size and shown, and the dock holds no mini window for it.

#[cfg(feature = "headless")]
mod minimise_restore_race_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    const TITLE: &str = "restore-race";
    /// Restore only once the minimise has run its course.
    const SETTLED: usize = usize::MAX;

    /// The window's drawn bounds in physical px, whether it is hidden, and
    /// whether the workspace model still lists it as minimised.
    fn window_state(handle: &HeadlessHandle) -> (f32, f32, bool, bool) {
        handle.query(|state| {
            let (id, _) = state
                .workspaces
                .windows_map
                .iter()
                .find(|(_, w)| w.xdg_title() == TITLE)
                .map(|(id, w)| (id.clone(), w.clone()))
                .expect("the window is known");
            let view = state.workspaces.get_window_view(&id).expect("a view");
            let bounds = view.window_layer.render_bounds_with_children_transformed();
            let listed = state
                .workspaces
                .with_model(|m| m.minimized_windows.iter().any(|(w, _)| *w == id));
            (
                bounds.width(),
                bounds.height(),
                view.window_layer.hidden(),
                listed,
            )
        })
    }

    /// Lets the genie's task and the engine's animations run their course.
    fn run_out(handle: &HeadlessHandle) {
        for _ in 0..8 {
            handle.settle(120);
            handle.wait(Duration::from_millis(100));
        }
    }

    fn check(delay_frames: usize) {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        let toplevel = client.create_toplevel(TITLE, 600, 400);
        handle.wait(Duration::from_millis(100));
        client.roundtrip().expect("roundtrip");
        toplevel.lock().unwrap().commit_frame();
        client.roundtrip().expect("roundtrip");
        handle.settle(300);
        handle.move_window(TITLE, 200, 150);
        let _ = client.roundtrip();
        handle.settle(300);
        let (w0, h0, hidden0, _) = window_state(&handle);
        assert!(
            !hidden0 && w0 > 100.0,
            "the window starts on screen ({w0}x{h0})"
        );

        handle.minimize_window(TITLE);
        if delay_frames == SETTLED {
            run_out(&handle);
        } else {
            for _ in 0..delay_frames {
                handle.tick(1.0 / 60.0);
            }
        }
        handle.unminimize_window(TITLE);
        run_out(&handle);

        let (w, h, hidden, listed) = window_state(&handle);
        assert!(
            !listed,
            "restored after {delay_frames} frames: still listed as minimised"
        );
        assert!(
            !hidden,
            "restored after {delay_frames} frames: the window is hidden"
        );
        assert!(
            (w - w0).abs() < 2.0 && (h - h0).abs() < 2.0,
            "restored after {delay_frames} frames: drawn {w}x{h}, not its own {w0}x{h0} \
             — left at mini scale"
        );
        handle.stop();
    }

    #[test]
    #[serial]
    fn a_restore_during_the_genie_brings_the_window_back_whole() {
        // Right away, and a few frames into the genie.
        for delay in [0, 6, 20] {
            check(delay);
        }
    }

    #[test]
    #[serial]
    fn a_restore_after_the_genie_brings_the_window_back_whole() {
        check(SETTLED);
    }

    /// Minimising a fullscreen window leaves fullscreen first: the user is
    /// back on the workspace the window came from, the fullscreen one is gone,
    /// and the window, in the dock, comes back windowed when restored.
    #[test]
    #[serial]
    fn minimising_a_fullscreen_window_leaves_fullscreen() {
        // Settled in fullscreen first, and halfway into it.
        for frames_in in [usize::MAX, 6] {
            let handle = HeadlessHandle::start(HeadlessConfig::default());
            let mut client = TestClient::connect(&handle.socket_name).expect("connect");
            let toplevel = client.create_toplevel(TITLE, 600, 400);
            handle.wait(Duration::from_millis(100));
            client.roundtrip().expect("roundtrip");
            toplevel.lock().unwrap().commit_frame();
            client.roundtrip().expect("roundtrip");
            handle.settle(300);
            handle.move_window(TITLE, 200, 150);
            let _ = client.roundtrip();
            run_out(&handle);
            let workspaces = || {
                handle.query(|state| {
                    let ows = state.workspaces.output_workspaces.values().next().unwrap();
                    (ows.current_workspace, ows.workspace_views.len())
                })
            };
            let (_, count0) = workspaces();
            let (w0, h0, _, _) = window_state(&handle);

            handle.fullscreen_window(TITLE);
            let _ = client.roundtrip();
            if frames_in == usize::MAX {
                run_out(&handle);
                assert!(
                    handle.window_is_fullscreen(TITLE),
                    "the window went fullscreen"
                );
            } else {
                for _ in 0..frames_in {
                    handle.tick(1.0 / 60.0);
                }
            }
            handle.with_state(|state| {
                let window = state
                    .workspaces
                    .windows_map
                    .values()
                    .find(|w| w.xdg_title() == TITLE)
                    .cloned()
                    .unwrap();
                state.minimize_window_element(&window);
            });
            let _ = client.roundtrip();
            run_out(&handle);

            let what = if frames_in == usize::MAX {
                "settled"
            } else {
                "mid-transition"
            };
            let (current, count) = workspaces();
            assert_eq!(current, 0, "{what}: back on the window's own workspace");
            assert_eq!(count, count0, "{what}: the fullscreen workspace is gone");
            let (_, h, hidden, listed) = window_state(&handle);
            assert!(
                listed && !hidden && h < h0 / 4.0,
                "{what}: the window is in the dock"
            );

            handle.unminimize_window(TITLE);
            let _ = client.roundtrip();
            run_out(&handle);
            let (w, h, hidden, listed) = window_state(&handle);
            assert!(!listed && !hidden, "{what}: restored");
            assert!(
                !handle.window_is_fullscreen(TITLE),
                "{what}: restored windowed"
            );
            assert!(
                (w - w0).abs() < 2.0 && (h - h0).abs() < 2.0,
                "{what}: restored at {w}x{h}, not its own {w0}x{h0}"
            );
            handle.stop();
        }
    }

    /// Clients killed together, as the live stress script ends, while their
    /// windows are minimised, mid-genie, fullscreen and leaving fullscreen:
    /// once the close fades are over no window layer is left in the scene.
    /// A stray one draws as a mini window in a corner of the screen.
    #[test]
    #[serial]
    fn windows_closed_in_every_state_leave_no_layer_behind() {
        // How many frames after the last action the clients die.
        for frames_before_kill in [0usize, 3, 12, 40] {
            let handle = HeadlessHandle::start(HeadlessConfig::default());
            let titles = [
                "min-settled",
                "min-genie",
                "fs-settled",
                "fs-then-min",
                "plain",
            ];
            let mut clients = Vec::new();
            for (i, title) in titles.iter().enumerate() {
                let mut client = TestClient::connect(&handle.socket_name).expect("connect");
                let toplevel = client.create_toplevel(title, 500, 340);
                handle.wait(Duration::from_millis(60));
                client.roundtrip().expect("roundtrip");
                toplevel.lock().unwrap().commit_frame();
                client.roundtrip().expect("roundtrip");
                handle.settle(120);
                handle.move_window(title, 80 + 60 * i as i32, 80 + 40 * i as i32);
                let _ = client.roundtrip();
                clients.push((client, toplevel));
            }
            run_out(&handle);
            let workspaces0 = handle.query(|state| {
                let ows = state.workspaces.output_workspaces.values().next().unwrap();
                ows.workspace_views.len()
            });
            let minimise = |title: &'static str| {
                handle.with_state(move |state| {
                    let window = state
                        .workspaces
                        .windows_map
                        .values()
                        .find(|w| w.xdg_title() == title)
                        .cloned()
                        .unwrap();
                    state.minimize_window_element(&window);
                });
            };
            let pump = |clients: &mut Vec<(TestClient, _)>| {
                for (client, _) in clients.iter_mut() {
                    let _ = client.roundtrip();
                }
            };

            let describe = |when: &str| {
                let s = handle.query(|state| {
                    let ows = state.workspaces.output_workspaces.values().next().unwrap();
                    let wss = ows
                        .workspace_views
                        .iter()
                        .zip(ows.spaces.iter())
                        .map(|(w, s)| {
                            format!(
                                "fs {} anim {} windows {}",
                                w.get_fullscreen_mode(),
                                w.get_fullscreen_animating(),
                                s.elements().count()
                            )
                        })
                        .collect::<Vec<_>>();
                    let wins = state
                        .workspaces
                        .windows_map
                        .values()
                        .map(|w| {
                            format!(
                                "{} fs {} ws {} min {}",
                                w.xdg_title(),
                                w.is_fullscreen(),
                                w.get_fullscreen_workspace(),
                                w.is_minimised()
                            )
                        })
                        .collect::<Vec<_>>();
                    format!("current {} {wss:?}\n  {wins:?}", ows.current_workspace)
                });
                eprintln!("[{frames_before_kill}] {when}: {s}");
            };
            describe("start");
            minimise("min-settled");
            run_out(&handle);
            handle.fullscreen_window("fs-settled");
            pump(&mut clients);
            run_out(&handle);
            describe("fs-settled fullscreen");
            handle.unfullscreen_window("fs-settled");
            pump(&mut clients);
            run_out(&handle);
            describe("fs-settled back");
            handle.fullscreen_window("fs-then-min");
            pump(&mut clients);
            run_out(&handle);
            describe("fs-then-min fullscreen");
            handle.fullscreen_window("fs-settled");
            pump(&mut clients);
            for _ in 0..6 {
                handle.tick(1.0 / 60.0);
            }
            describe("fs-settled going fullscreen");
            minimise("fs-then-min");
            describe("fs-then-min minimised");
            minimise("min-genie");
            pump(&mut clients);
            for _ in 0..frames_before_kill {
                handle.tick(1.0 / 60.0);
            }

            let views = handle.query(|state| {
                state
                    .workspaces
                    .windows_map
                    .iter()
                    .map(|(id, w)| {
                        (
                            w.xdg_title(),
                            state.workspaces.get_window_view(id).expect("a view"),
                        )
                    })
                    .collect::<Vec<_>>()
            });
            assert_eq!(views.len(), titles.len());
            describe("before kill");

            // Every client dies at once.
            drop(clients);
            for _ in 0..3 {
                run_out(&handle);
            }
            describe("after kill");

            let leftovers = handle.query(move |state| {
                let engine = &state.layers_engine;
                views
                    .iter()
                    .filter(|(_, view)| view.is_alive())
                    .map(|(title, view)| {
                        let mut chain = Vec::new();
                        let mut node = engine.scene_get_node_parent(view.window_layer.id());
                        while let Some(n) = node {
                            let key = engine.get_layer(&n).map(|l| l.key()).unwrap_or_default();
                            chain.push(if key.is_empty() {
                                format!("{n:?}")
                            } else {
                                key
                            });
                            node = engine.scene_get_node_parent(n);
                        }
                        let b = view.window_layer.render_bounds_with_children_transformed();
                        format!(
                            "{title}: hidden {} at {:.0},{:.0} {:.0}x{:.0}, parents {chain:?}",
                            view.window_layer.hidden(),
                            b.x(),
                            b.y(),
                            b.width(),
                            b.height()
                        )
                    })
                    .collect::<Vec<_>>()
            });
            let (ws, workspaces, minimised) = handle.query(|state| {
                let ows = state.workspaces.output_workspaces.values().next().unwrap();
                (
                    ows.current_workspace,
                    ows.workspace_views.len(),
                    state.workspaces.with_model(|m| m.minimized_windows.len()),
                )
            });
            handle.stop();
            assert!(
                leftovers.is_empty(),
                "killed {frames_before_kill} frames in: window layers left in the scene:\n{}",
                leftovers.join("\n")
            );
            assert_eq!(
                minimised, 0,
                "killed {frames_before_kill} frames in: dock still lists windows"
            );
            assert_eq!(
                (ws, workspaces),
                (0, workspaces0),
                "killed {frames_before_kill} frames in: fullscreen workspace left behind"
            );
        }
    }

    /// A minimised window asked to go fullscreen or maximized — `wlrctl
    /// toplevel fullscreen` does that to every match, minimised or not — stays
    /// in its dock drawer, and comes back windowed when restored.
    #[test]
    #[serial]
    fn a_minimised_window_stays_in_the_dock_when_asked_to_grow() {
        for action in ["fullscreen", "maximize"] {
            let handle = HeadlessHandle::start(HeadlessConfig::default());
            let mut client = TestClient::connect(&handle.socket_name).expect("connect");
            let toplevel = client.create_toplevel(TITLE, 600, 400);
            handle.wait(Duration::from_millis(100));
            client.roundtrip().expect("roundtrip");
            toplevel.lock().unwrap().commit_frame();
            client.roundtrip().expect("roundtrip");
            handle.settle(300);
            handle.move_window(TITLE, 200, 150);
            let _ = client.roundtrip();
            run_out(&handle);
            let (w0, h0, _, _) = window_state(&handle);

            handle.minimize_window(TITLE);
            run_out(&handle);
            let ows_count = || {
                handle.query(|state| {
                    let ows = state.workspaces.output_workspaces.values().next().unwrap();
                    (ows.current_workspace, ows.workspace_views.len())
                })
            };
            let before = ows_count();
            if action == "fullscreen" {
                handle.fullscreen_window(TITLE);
            } else {
                handle.with_state(|state| {
                    use smithay::wayland::shell::xdg::XdgShellHandler;
                    let toplevel = state
                        .workspaces
                        .windows_map
                        .values()
                        .find(|w| w.xdg_title() == TITLE)
                        .and_then(|w| w.toplevel().cloned())
                        .unwrap();
                    state.maximize_request(toplevel);
                });
            }
            let _ = client.roundtrip();
            run_out(&handle);

            let (_, h, hidden, listed) = window_state(&handle);
            let y = handle.query(|state| {
                let id = state
                    .workspaces
                    .windows_map
                    .iter()
                    .find(|(_, w)| w.xdg_title() == TITLE)
                    .map(|(id, _)| id.clone())
                    .unwrap();
                let view = state.workspaces.get_window_view(&id).unwrap();
                view.window_layer
                    .render_bounds_with_children_transformed()
                    .y()
            });
            assert!(
                listed && !hidden && h < h0 / 4.0 && y > 1080.0 / 2.0,
                "{action}: the mini window ({h} high at y {y}) left the dock"
            );
            assert_eq!(ows_count(), before, "{action}: no workspace came or went");

            handle.unminimize_window(TITLE);
            let _ = client.roundtrip();
            run_out(&handle);
            let (w, h, hidden, listed) = window_state(&handle);
            assert!(!listed && !hidden, "{action}: restored");
            assert!(
                (w - w0).abs() < 2.0 && (h - h0).abs() < 2.0,
                "{action}: restored at {w}x{h}, not its own {w0}x{h0}"
            );
            handle.stop();
        }
    }

    /// Going fullscreen parks the window above everything for the transition
    /// and moves it into its fullscreen workspace when the transition ends. A
    /// window minimised before then is in its dock drawer by that time, and
    /// stays there.
    #[test]
    #[serial]
    fn a_window_minimised_while_going_fullscreen_stays_in_the_dock() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        let toplevel = client.create_toplevel(TITLE, 600, 400);
        handle.wait(Duration::from_millis(100));
        client.roundtrip().expect("roundtrip");
        toplevel.lock().unwrap().commit_frame();
        client.roundtrip().expect("roundtrip");
        handle.settle(300);
        let (_, h0, _, _) = window_state(&handle);

        handle.fullscreen_window(TITLE);
        let _ = client.roundtrip();
        for _ in 0..6 {
            handle.tick(1.0 / 60.0);
        }
        handle.minimize_window(TITLE);
        run_out(&handle);

        let (w, h, hidden, listed) = window_state(&handle);
        let y = handle.query(|state| {
            let id = state
                .workspaces
                .windows_map
                .iter()
                .find(|(_, w)| w.xdg_title() == TITLE)
                .map(|(id, _)| id.clone())
                .unwrap();
            let view = state.workspaces.get_window_view(&id).unwrap();
            view.window_layer
                .render_bounds_with_children_transformed()
                .y()
        });
        // The headless output's mode, in physical px, as the bounds are.
        let screen_h = 1080.0_f32;
        assert!(
            listed && !hidden,
            "the window is minimised and shown in the dock"
        );
        assert!(
            h < h0 / 4.0 && y > screen_h / 2.0,
            "the mini window ({w}x{h} at y {y}) is not in the dock at the bottom \
             of the {screen_h}px screen"
        );
        handle.stop();
    }
}
