//! The compositor's windows as `org.otto.Shell1` lists them, each with the
//! process that owns it, which the foreign-toplevel protocol doesn't carry.
//!
//! The music island uses it to bring a player's window forward by process
//! rather than by guessing from names.

use otto_kit::dbus::shell::ShellProxyBlocking;
use serde_json::Value;
use zbus::blocking::Connection;

/// One window in the compositor's tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellWindow {
    /// The window's `id`, which `[con_id=…]` matches.
    pub con_id: u64,
    /// The process that owns the window, when the compositor knows it.
    pub pid: Option<u32>,
    pub app_id: String,
    pub title: String,
}

/// Every window, read from `GetTree`.
pub fn windows(conn: &Connection) -> zbus::Result<Vec<ShellWindow>> {
    let tree = ShellProxyBlocking::new(conn)?.get_tree()?;
    Ok(parse_tree(&tree))
}

/// Focus the window with `con_id`, switching to its workspace. Returns
/// whether the compositor found it.
pub fn focus(conn: &Connection, con_id: u64) -> zbus::Result<bool> {
    let results =
        ShellProxyBlocking::new(conn)?.run_command(&format!("[con_id={con_id}] focus"))?;
    Ok(results.first().is_some_and(|(success, _)| *success))
}

/// The windows in a `GetTree` answer, in tree order. Anything unreadable
/// reads as no windows.
pub fn parse_tree(tree: &str) -> Vec<ShellWindow> {
    let mut out = Vec::new();
    if let Ok(root) = serde_json::from_str::<Value>(tree) {
        collect(&root, &mut out);
    }
    out
}

fn collect(node: &Value, out: &mut Vec<ShellWindow>) {
    // Windows are the nodes with an `app_id`; outputs, workspaces and split
    // containers have none.
    if let (Some(app_id), Some(con_id)) = (
        node.get("app_id").and_then(Value::as_str),
        node.get("id").and_then(Value::as_u64),
    ) {
        out.push(ShellWindow {
            con_id,
            pid: node
                .get("pid")
                .and_then(Value::as_u64)
                .and_then(|pid| u32::try_from(pid).ok()),
            app_id: app_id.to_string(),
            title: node
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    for key in ["nodes", "floating_nodes"] {
        for child in node
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            collect(child, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_are_read_from_tiled_and_floating_nodes() {
        let tree = r#"{
            "id": 1, "type": "root",
            "nodes": [{
                "id": 2, "type": "output",
                "nodes": [{
                    "id": 3, "type": "workspace",
                    "nodes": [{
                        "id": 4, "type": "con", "layout": "splith",
                        "nodes": [
                            {"id": 10, "type": "con", "name": "Hacker News - Google Chrome",
                             "app_id": "google-chrome", "pid": 372737, "nodes": []}
                        ]
                    }],
                    "floating_nodes": [
                        {"id": 11, "type": "con", "name": "Files", "app_id": "otto-files",
                         "pid": null, "nodes": []}
                    ]
                }]
            }]
        }"#;
        assert_eq!(
            parse_tree(tree),
            vec![
                ShellWindow {
                    con_id: 10,
                    pid: Some(372737),
                    app_id: "google-chrome".into(),
                    title: "Hacker News - Google Chrome".into(),
                },
                ShellWindow {
                    con_id: 11,
                    pid: None,
                    app_id: "otto-files".into(),
                    title: "Files".into(),
                },
            ]
        );
    }

    #[test]
    fn an_unreadable_tree_has_no_windows() {
        assert!(parse_tree("not json").is_empty());
    }
}
