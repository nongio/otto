//! End-to-end tests for the scripting surface.
//!
//! Drives `run_command` — the entry point `org.otto.Shell1`'s `RunCommand`
//! and `otto-msg` both land on — and asserts on the geometry and on
//! `tree_json`, so a command's effect is checked where a script would see it.
//!
//! The D-Bus round trip itself is not tested here: the headless harness has no
//! session bus to claim `org.otto.Shell1` on, and a test that started one
//! would be testing zbus rather than Otto. `src/shell_service.rs` is a thin
//! shim over exactly these calls.

#[cfg(feature = "headless")]
mod tiling_scripting_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serde_json::Value;
    use serial_test::serial;
    use std::time::Duration;

    /// One window, with the client that owns it kept alive.
    struct Window {
        #[allow(dead_code)]
        client: TestClient,
    }

    fn spawn(handle: &HeadlessHandle, title: &str) -> Window {
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");
        let _toplevel = client.create_toplevel(title, 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        Window { client }
    }

    fn setup(titles: &[&str]) -> (HeadlessHandle, Vec<Window>) {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let windows: Vec<Window> = titles.iter().map(|t| spawn(&handle, t)).collect();
        handle.settle(300);
        (handle, windows)
    }

    /// Run a command string and insist every command in it worked.
    fn run(handle: &HeadlessHandle, command: &str) {
        for (index, result) in handle.run_command(command).into_iter().enumerate() {
            result.unwrap_or_else(|err| panic!("`{command}` command {index} failed: {err}"));
        }
        handle.settle(600);
    }

    fn cell(handle: &HeadlessHandle, title: &str) -> (i32, i32, i32, i32) {
        handle
            .tiling_cell_rects()
            .into_iter()
            .find(|(t, _)| t == title)
            .unwrap_or_else(|| panic!("{title} should be a tile"))
            .1
    }

    /// Every node in a `GetTree` answer, root first.
    fn nodes(tree: &Value) -> Vec<Value> {
        let mut out = vec![tree.clone()];
        for key in ["nodes", "floating_nodes"] {
            if let Some(children) = tree[key].as_array() {
                for child in children {
                    out.extend(nodes(child));
                }
            }
        }
        out
    }

    fn node_named(tree: &Value, name: &str) -> Option<Value> {
        nodes(tree).into_iter().find(|node| node["name"] == name)
    }

    // ── Mode ─────────────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn tiling_toggle_turns_the_workspace_into_a_tree() {
        let (handle, windows) = setup(&["cmd-a", "cmd-b"]);
        handle.focus_window("cmd-a");
        assert!(!handle.workspace_tiling_enabled());

        run(&handle, "tiling toggle");
        assert!(handle.workspace_tiling_enabled());
        assert_eq!(handle.tiling_tree_leaves().len(), 2);

        // `enable` on an already tiling workspace is not a toggle.
        run(&handle, "tiling enable");
        assert!(handle.workspace_tiling_enabled());

        run(&handle, "tiling disable");
        assert!(!handle.workspace_tiling_enabled());

        drop(windows);
        handle.stop();
    }

    // ── Focus and movement ───────────────────────────────────────────────

    #[test]
    #[serial]
    fn focus_right_and_move_left_walk_the_tree() {
        let (handle, windows) = setup(&["cmd-a", "cmd-b"]);
        handle.focus_window("cmd-a");
        run(&handle, "tiling toggle");

        let order = handle.tiling_tree_leaves();
        assert_eq!(order.len(), 2);
        handle.focus_window(&order[0]);
        handle.settle(100);

        run(&handle, "focus right");
        assert_eq!(handle.focused_window_title(), Some(order[1].clone()));

        run(&handle, "move left");
        assert_eq!(
            handle.tiling_tree_leaves(),
            vec![order[1].clone(), order[0].clone()],
            "the two tiles traded places"
        );

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn focus_parent_and_child_walk_up_and_back_down() {
        let (handle, windows) = setup(&["cmd-a", "cmd-b"]);
        handle.focus_window("cmd-a");
        run(&handle, "tiling toggle");

        // Up to the root container, and no further: focus never leaves the
        // workspace.
        run(&handle, "focus parent");
        let container = nodes(&handle.tree_json())
            .into_iter()
            .find(|node| node["type"] == "con" && node["focused"] == Value::Bool(true));
        assert!(
            container.is_some(),
            "a container is focused after `focus parent`"
        );
        assert!(
            handle.run_command("focus parent")[0].is_err(),
            "there is nothing above the workspace's root"
        );

        run(&handle, "focus child");
        let still_a_container = nodes(&handle.tree_json()).into_iter().any(|node| {
            node["type"] == "con"
                && node["layout"] != "none"
                && node["focused"] == Value::Bool(true)
        });
        assert!(!still_a_container, "focus came back down to a window");

        drop(windows);
        handle.stop();
    }

    // ── Splitting and resizing ───────────────────────────────────────────

    #[test]
    #[serial]
    fn split_v_makes_the_next_window_a_row_below() {
        let (handle, mut windows) = setup(&["cmd-a", "cmd-b"]);
        handle.focus_window("cmd-b");
        run(&handle, "tiling toggle");
        let before = cell(&handle, "cmd-b");

        run(&handle, "split v");
        windows.push(spawn(&handle, "cmd-c"));
        handle.settle(600);

        let b = cell(&handle, "cmd-b");
        let c = cell(&handle, "cmd-c");
        assert_eq!(
            (b.0, b.2),
            (before.0, before.2),
            "the split kept the column's width"
        );
        assert!(c.1 > b.1, "the new cell took the bottom half: {b:?} {c:?}");

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn resize_grow_width_10_ppt_takes_from_the_neighbour() {
        let (handle, windows) = setup(&["cmd-a", "cmd-b"]);
        handle.focus_window("cmd-a");
        run(&handle, "tiling toggle");

        let order = handle.tiling_tree_leaves();
        handle.focus_window(&order[0]);
        handle.settle(100);
        let before = cell(&handle, &order[0]);
        let neighbour_before = cell(&handle, &order[1]);

        run(&handle, "resize grow width 10 ppt");

        let after = cell(&handle, &order[0]);
        let neighbour = cell(&handle, &order[1]);
        // Ten percent of the container, which is the whole usable width.
        let expected = before.2 + (before.2 + neighbour_before.2) / 10;
        assert!(
            (after.2 - expected).abs() <= 8,
            "the cell grew by about a tenth of the container: {} → {} (wanted {expected})",
            before.2,
            after.2
        );
        assert!(
            neighbour.2 < neighbour_before.2,
            "the neighbour gave the space up: {} → {}",
            neighbour_before.2,
            neighbour.2
        );
        assert_eq!(
            after.2 + neighbour.2,
            before.2 + neighbour_before.2,
            "the two still fill the same width"
        );

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn layout_splitv_turns_the_container() {
        let (handle, windows) = setup(&["cmd-a", "cmd-b"]);
        handle.focus_window("cmd-a");
        run(&handle, "tiling toggle");

        let order = handle.tiling_tree_leaves();
        let left = cell(&handle, &order[0]);
        let right = cell(&handle, &order[1]);
        assert!(right.0 > left.0, "they start side by side");

        run(&handle, "layout splitv");

        let top = cell(&handle, &order[0]);
        let bottom = cell(&handle, &order[1]);
        assert!(
            bottom.1 > top.1 && bottom.0 == top.0,
            "the container now stacks them: {top:?} {bottom:?}"
        );

        drop(windows);
        handle.stop();
    }

    // ── Workspaces ───────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn move_container_to_workspace_2_takes_the_window_with_it() {
        let (handle, windows) = setup(&["cmd-a", "cmd-b"]);
        handle.focus_window("cmd-a");
        run(&handle, "tiling toggle");
        assert_eq!(handle.tiling_tree_leaves().len(), 2);

        handle.focus_window("cmd-b");
        handle.settle(100);
        run(&handle, "move container to workspace 2");

        assert_eq!(
            handle.tiling_tree_leaves(),
            vec!["cmd-a".to_string()],
            "the moved window left the tree behind"
        );

        // The window is on workspace 2, whether or not that workspace existed
        // before the command.
        handle.set_workspace(1);
        handle.settle(600);
        let titles = handle.window_stack_titles();
        assert!(
            titles.contains(&"cmd-b".to_string()),
            "cmd-b is on workspace 2: {titles:?}"
        );

        drop(windows);
        handle.stop();
    }

    // ── Gaps ─────────────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn gaps_on_one_workspace_leave_another_alone() {
        let (handle, mut windows) = setup(&["gap-a", "gap-b"]);
        handle.focus_window("gap-a");
        run(&handle, "tiling toggle");
        let before = cell(&handle, "gap-a");

        // A second workspace with its own pair of tiles.
        handle.set_workspace(1);
        handle.settle(600);
        windows.push(spawn(&handle, "gap-c"));
        windows.push(spawn(&handle, "gap-d"));
        handle.focus_window("gap-c");
        run(&handle, "tiling toggle");
        let other_before = cell(&handle, "gap-c");
        assert!(handle.workspace_gap_override().is_none());

        // Close the gaps on this workspace only.
        run(&handle, "gaps inner 0");
        run(&handle, "gaps outer 0");
        assert_eq!(handle.workspace_gap_override(), Some((0, 0)));

        let (zx, zy, zw, zh) = handle.usable_zone();
        let cells = handle.tiling_cell_rects();
        let first = cells.first().expect("this workspace tiles").1;
        let last = cells.last().expect("this workspace tiles").1;
        assert_eq!(
            (first.0, first.1, first.3),
            (zx, zy, zh),
            "with no gaps the first cell starts at the usable area's corner"
        );
        assert_eq!(
            last.0 + last.2,
            zx + zw,
            "and the last one ends at its far edge"
        );
        assert_ne!(
            cell(&handle, "gap-c").2,
            other_before.2,
            "the override changed this workspace's layout"
        );

        // The first workspace never asked for it.
        handle.set_workspace(0);
        handle.settle(600);
        assert!(
            handle.workspace_gap_override().is_none(),
            "the override belongs to the workspace it was set on"
        );
        assert_eq!(
            cell(&handle, "gap-a"),
            before,
            "the other workspace's layout is untouched"
        );
        assert!(
            before.0 > zx,
            "sanity: the first workspace still has its outer gap ({before:?} inside {zx})"
        );

        drop(windows);
        handle.stop();
    }

    /// A persisted setting must land in the session's own throwaway config
    /// directory and nowhere else — least of all in the developer's
    /// `~/.config/otto/config.toml`, which an earlier version of this test
    /// really did rewrite.
    #[test]
    #[serial]
    fn a_persisted_override_stays_inside_the_test_session() {
        let real = user_config_file();
        let before = real.as_ref().map(|path| digest(path));

        let (handle, windows) = setup(&["persist-a", "persist-b"]);
        handle.focus_window("persist-a");
        run(&handle, "tiling toggle");
        run(&handle, "gaps inner 0 current");

        let written = handle.config_root.join("otto").join("config.toml");
        assert!(
            written.is_file(),
            "the override should be persisted under the session's config root ({})",
            handle.config_root.display()
        );
        let text = std::fs::read_to_string(&written).expect("the session's own config");
        assert!(
            text.contains("[workspaces.entries"),
            "the gap override should be in the session's config, not somewhere else:\n{text}"
        );

        assert_eq!(
            real.as_ref().map(|path| digest(path)),
            before,
            "the user's own config.toml must be byte-identical before and after"
        );

        drop(windows);
        handle.stop();
        assert!(
            !handle_config_root_exists(&written),
            "the session's config directory is cleaned up on stop"
        );
    }

    /// Everything a workspace persists lands in one record: the mode the
    /// toggle set and the gaps the command set, in a single
    /// `[workspaces.entries."…"]` table, with neither of the two tables that
    /// came before it left in the file.
    #[test]
    #[serial]
    fn one_record_holds_the_workspace_mode_and_its_gaps() {
        let (handle, windows) = setup(&["record-a", "record-b"]);
        handle.focus_window("record-a");
        run(&handle, "tiling toggle");
        run(&handle, "gaps inner 0 current");
        run(&handle, "gaps outer 2 current");

        let written = handle.config_root.join("otto").join("config.toml");
        let text = std::fs::read_to_string(&written).expect("the session's own config");

        let tables: Vec<&str> = text
            .lines()
            .filter(|line| line.trim_start().starts_with("[workspaces.entries."))
            .collect();
        assert_eq!(
            tables.len(),
            1,
            "exactly one workspace has a record:\n{text}"
        );

        // The record is the whole of what the workspace persisted.
        let record: toml::Value = toml::from_str(&text).expect("the file stays parsable");
        let entries = record["workspaces"]["entries"]
            .as_table()
            .expect("entries is a table");
        let (_, entry) = entries.iter().next().expect("one record");
        assert_eq!(entry["tiling"].as_bool(), Some(true), "{text}");
        assert_eq!(entry["inner_gap"].as_integer(), Some(0), "{text}");
        assert_eq!(entry["outer_gap"].as_integer(), Some(2), "{text}");

        let workspaces = record["workspaces"].as_table().expect("a table");
        assert!(
            !workspaces.contains_key("names") && !workspaces.contains_key("gaps"),
            "the tables the record replaced must not be written:\n{text}"
        );

        drop(windows);
        handle.stop();
    }

    fn handle_config_root_exists(written: &std::path::Path) -> bool {
        written.exists()
    }

    /// The real user config, resolved exactly the way `src/config` resolves
    /// it — so this compares the file the compositor would otherwise write.
    fn user_config_file() -> Option<std::path::PathBuf> {
        let dir = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|home| std::path::PathBuf::from(home).join(".config"))
            })?;
        Some(dir.join("otto").join("config.toml"))
    }

    /// A cheap content digest — `None` when the file is not there, so "absent
    /// before and absent after" also compares equal.
    fn digest(path: &std::path::Path) -> Option<(u64, u64)> {
        let bytes = std::fs::read(path).ok()?;
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in &bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Some((bytes.len() as u64, hash))
    }

    // ── GetTree ──────────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn the_tree_reports_the_cells_it_laid_out() {
        let (handle, windows) = setup(&["cmd-a", "cmd-b"]);
        handle.focus_window("cmd-a");
        run(&handle, "tiling toggle");

        let tree = handle.tree_json();
        assert_eq!(tree["type"], "root");
        assert_eq!(tree["nodes"][0]["type"], "output");

        let workspace = tree["nodes"][0]["nodes"][0].clone();
        assert_eq!(workspace["type"], "workspace");
        assert_eq!(
            workspace["layout"], "splith",
            "two tiles side by side is a horizontal split"
        );
        assert!(workspace["gaps"].is_null(), "no override to report");

        for title in ["cmd-a", "cmd-b"] {
            let node = node_named(&tree, title).unwrap_or_else(|| panic!("{title} is in the tree"));
            assert_eq!(node["type"], "con");
            assert_eq!(node["layout"], "none");
            assert_eq!(node["urgent"], Value::Bool(false));
            let rect = &node["rect"];
            let want = cell(&handle, title);
            assert_eq!(
                (
                    rect["x"].as_i64().unwrap() as i32,
                    rect["y"].as_i64().unwrap() as i32,
                    rect["width"].as_i64().unwrap() as i32,
                    rect["height"].as_i64().unwrap() as i32,
                ),
                want,
                "{title}'s node carries its cell"
            );
        }

        let focused: Vec<String> = nodes(&tree)
            .into_iter()
            .filter(|node| node["focused"] == Value::Bool(true) && node["layout"] == "none")
            .map(|node| node["name"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(
            focused.len(),
            1,
            "exactly one window is focused: {focused:?}"
        );

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn a_floating_workspace_lists_its_windows_as_floating_nodes() {
        let (handle, windows) = setup(&["cmd-a", "cmd-b"]);
        handle.settle(300);

        let tree = handle.tree_json();
        let workspace = tree["nodes"][0]["nodes"][0].clone();
        assert!(
            workspace["nodes"].as_array().is_some_and(|n| n.is_empty()),
            "nothing is in a tree on a floating workspace"
        );
        let floating: Vec<String> = workspace["floating_nodes"]
            .as_array()
            .expect("floating_nodes is a list")
            .iter()
            .map(|node| node["name"].as_str().unwrap_or_default().to_string())
            .collect();
        for title in ["cmd-a", "cmd-b"] {
            assert!(floating.contains(&title.to_string()), "{floating:?}");
        }

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn get_workspaces_and_get_outputs_answer_in_i3s_shapes() {
        let (handle, windows) = setup(&["cmd-a"]);
        handle.settle(300);

        let workspaces = handle.workspaces_json();
        let list = workspaces.as_array().expect("an array");
        assert!(!list.is_empty());
        let visible: Vec<&Value> = list
            .iter()
            .filter(|w| w["visible"] == Value::Bool(true))
            .collect();
        assert_eq!(visible.len(), 1, "one workspace is visible per output");
        for key in ["num", "name", "visible", "focused", "output", "rect"] {
            assert!(!list[0][key].is_null(), "a workspace carries `{key}`");
        }

        let outputs = handle.outputs_json();
        let list = outputs.as_array().expect("an array");
        assert_eq!(list.len(), 1, "the headless backend has one output");
        assert_eq!(list[0]["active"], Value::Bool(true));
        for key in ["name", "current_workspace", "rect"] {
            assert!(!list[0][key].is_null(), "an output carries `{key}`");
        }

        drop(windows);
        handle.stop();
    }

    // ── Errors ───────────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn a_bad_command_comes_back_with_the_character_it_stumbled_on() {
        let (handle, windows) = setup(&["cmd-a"]);

        let results = handle.run_command("focus left; frobnicate");
        assert_eq!(results.len(), 1, "a parse error abandons the whole string");
        let error = results[0].clone().expect_err("it did not parse");
        assert!(
            error.contains("Unknown/invalid command 'frobnicate'"),
            "{error}"
        );
        assert!(error.contains("at character 12"), "{error}");

        // Understood, but not built yet — said so rather than ignored.
        let error = handle.run_command("layout tabbed")[0]
            .clone()
            .expect_err("not supported yet");
        assert!(
            error.contains("does not support 'layout tabbed'"),
            "{error}"
        );

        // Parsed, and refused at run time: one result, not a parse failure.
        let results = handle.run_command("focus left; resize grow width 10 ppt");
        assert_eq!(results.len(), 2, "both commands were tried: {results:?}");
        assert!(
            results[1].is_err(),
            "a floating workspace has nothing to resize"
        );

        drop(windows);
        handle.stop();
    }

    // ── The floating layer ───────────────────────────────────────────────

    #[test]
    #[serial]
    fn a_child_toplevel_floats_instead_of_joining_the_tree() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");
        let parent = client.create_toplevel("cmd-parent", 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        handle.focus_window("cmd-parent");
        run(&handle, "tiling toggle");
        assert_eq!(handle.tiling_tree_leaves(), vec!["cmd-parent".to_string()]);

        // A dialog: it has a parent, so it floats above the tiles and the
        // layout carries on as if it were not there.
        let _dialog = client.create_child_toplevel("cmd-dialog", &parent, 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(400);

        assert_eq!(
            handle.tiling_tree_leaves(),
            vec!["cmd-parent".to_string()],
            "the dialog is not a tile"
        );
        let tree = handle.tree_json();
        let workspace = tree["nodes"][0]["nodes"][0].clone();
        let floating: Vec<String> = workspace["floating_nodes"]
            .as_array()
            .expect("floating_nodes is a list")
            .iter()
            .map(|node| node["name"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(floating.contains(&"cmd-dialog".to_string()), "{floating:?}");
        assert!(node_named(&workspace, "cmd-dialog").is_some());

        drop(client);
        handle.stop();
    }

    /// The live crash: a workspace that tiles from the start, a window
    /// opened straight into it (no floating rect to go back to), floated
    /// while it is the only tile.
    #[test]
    #[serial]
    fn floating_a_lone_tile_opened_into_the_tree_does_not_crash() {
        let (handle, _none) = setup(&[]);
        run(&handle, "tiling enable");
        handle.set_tiling_decoration("minimal");
        handle.settle(200);
        let a = spawn(&handle, "lone-a");
        handle.settle(300);
        handle.focus_window("lone-a");
        handle.settle(100);
        assert_eq!(handle.tiling_tree_leaves(), vec!["lone-a".to_string()]);
        run(&handle, "floating toggle");
        assert!(handle.tiling_tree_leaves().is_empty());
        assert!(handle.window_floating_rect("lone-a").is_some());
        run(&handle, "floating toggle");
        assert_eq!(handle.tiling_tree_leaves(), vec!["lone-a".to_string()]);

        // Two windows opened into the tree, float the second, then the first.
        let b = spawn(&handle, "lone-b");
        handle.settle(300);
        handle.focus_window("lone-b");
        handle.settle(100);
        run(&handle, "floating toggle");
        handle.focus_window("lone-a");
        handle.settle(100);
        run(&handle, "floating toggle");
        assert!(handle.tiling_tree_leaves().is_empty());
        // Nothing tiled: the layer switch says so instead of doing nothing.
        assert!(handle.run_command("focus mode_toggle")[0].is_err());
        drop((a, b));
        handle.stop();
    }

    #[test]
    #[serial]
    fn floating_toggle_takes_a_tile_out_of_the_tree_and_puts_it_back() {
        let (handle, windows) = setup(&["cmd-a", "cmd-b"]);
        handle.focus_window("cmd-a");
        run(&handle, "tiling toggle");
        assert_eq!(handle.tiling_tree_leaves().len(), 2);

        let (zx, zy, zw, zh) = handle.usable_zone();
        handle.focus_window("cmd-b");
        handle.settle(100);
        run(&handle, "floating toggle");

        assert_eq!(
            handle.tiling_tree_leaves(),
            vec!["cmd-a".to_string()],
            "the floated window left the tree"
        );
        // The survivor fills the area the pair shared.
        let (x, y, w, h) = cell(&handle, "cmd-a");
        assert!(x >= zx && y >= zy, "({x},{y}) is inside ({zx},{zy})");
        assert!(
            w >= zw - 40 && h >= zh - 40,
            "a lone tile fills the usable area: {w}x{h} of {zw}x{zh}"
        );

        // And back in, at the focused position.
        handle.focus_window("cmd-b");
        handle.settle(100);
        run(&handle, "floating toggle");
        let leaves = handle.tiling_tree_leaves();
        assert_eq!(leaves.len(), 2, "{leaves:?}");
        assert!(leaves.contains(&"cmd-b".to_string()), "{leaves:?}");
        let (_, _, w, _) = cell(&handle, "cmd-a");
        assert!(w < zw - 40, "two tiles share the width again: {w} of {zw}");

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn floating_enable_and_disable_are_not_toggles() {
        let (handle, windows) = setup(&["cmd-a", "cmd-b"]);
        handle.focus_window("cmd-a");
        run(&handle, "tiling toggle");
        handle.focus_window("cmd-b");
        handle.settle(100);

        run(&handle, "floating enable");
        assert_eq!(handle.tiling_tree_leaves(), vec!["cmd-a".to_string()]);
        // Already floating: `enable` says so rather than tiling it again.
        assert!(handle.run_command("floating enable")[0].is_err());
        assert_eq!(handle.tiling_tree_leaves(), vec!["cmd-a".to_string()]);

        run(&handle, "floating disable");
        assert_eq!(handle.tiling_tree_leaves().len(), 2);

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn a_floating_window_stays_above_the_tiles() {
        let (handle, windows) = setup(&["cmd-a", "cmd-b", "cmd-c"]);
        handle.focus_window("cmd-a");
        run(&handle, "tiling toggle");

        handle.focus_window("cmd-c");
        handle.settle(100);
        run(&handle, "floating toggle");
        assert_eq!(handle.top_window_title(), Some("cmd-c".to_string()));

        // Focusing a tile must not lift it over the floating window.
        handle.focus_window("cmd-a");
        handle.settle(300);
        assert_eq!(
            handle.window_stack_titles().last().cloned(),
            Some("cmd-c".to_string()),
            "stack: {:?}",
            handle.window_stack_titles()
        );

        // Nor must a directional focus, which raises as it goes.
        run(&handle, "focus right");
        assert_eq!(
            handle.window_stack_titles().last().cloned(),
            Some("cmd-c".to_string()),
            "stack: {:?}",
            handle.window_stack_titles()
        );

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn focus_mode_toggle_crosses_between_the_layers() {
        let (handle, windows) = setup(&["cmd-a", "cmd-b"]);
        handle.focus_window("cmd-a");
        run(&handle, "tiling toggle");

        handle.focus_window("cmd-b");
        handle.settle(100);
        run(&handle, "floating toggle");
        assert_eq!(handle.focused_window_title(), Some("cmd-b".to_string()));

        // Down into the tree…
        run(&handle, "focus mode_toggle");
        assert_eq!(handle.focused_window_title(), Some("cmd-a".to_string()));
        // …and back up to the floating layer.
        run(&handle, "focus mode_toggle");
        assert_eq!(handle.focused_window_title(), Some("cmd-b".to_string()));

        // The named halves reach the same two windows.
        run(&handle, "focus tiling");
        assert_eq!(handle.focused_window_title(), Some("cmd-a".to_string()));
        run(&handle, "focus floating");
        assert_eq!(handle.focused_window_title(), Some("cmd-b".to_string()));

        drop(windows);
        handle.stop();
    }
}
