//! What a session's prompts attached, and the reads that attaching allows.
//!
//! Attaching a file to a prompt is the person handing it to the agent, so the
//! agent reading it needs no second yes. A permission request is answered
//! here, without asking anyone, when it is a read ([`ToolKind::Read`]) and
//! everything it names is an attached file, or lies inside an attached folder.
//! Anything else, writes included, goes the usual way.
//!
//! Paths are compared as the filesystem resolves them: symbolic links are
//! followed on both sides, so a link inside an attached folder that points
//! out of it covers nothing, and a path that does not exist covers nothing
//! either. The host keeps what a session attached for as long as it runs, and
//! hands it to each agent process the session starts.

use std::path::{Path, PathBuf};

use agent_client_protocol::schema::v1::{
    PermissionOptionKind, RequestPermissionRequest, RequestPermissionResponse, ToolKind,
};

use crate::agent::Attachment;
use crate::dialog;

/// The files and folders a session's prompts attached, resolved.
#[derive(Debug, Default)]
pub struct Attached {
    /// Each attachment's resolved path, and whether it is a folder.
    paths: Vec<(PathBuf, bool)>,
}

impl Attached {
    /// Records the local files among `attachments`.
    ///
    /// An attachment that is not a `file://` URI, or names nothing on disk,
    /// is skipped.
    pub fn add(&mut self, attachments: &[Attachment]) {
        for attachment in attachments {
            let Some(path) = crate::uri::to_path(&attachment.uri) else {
                continue;
            };
            let Ok(resolved) = path.canonicalize() else {
                tracing::debug!(uri = %attachment.uri, "an attachment that is not on disk");
                continue;
            };
            let folder = resolved.is_dir();
            if !self.paths.iter().any(|(known, _)| *known == resolved) {
                self.paths.push((resolved, folder));
            }
        }
    }

    /// Whether `path` is attached, or lies inside an attached folder.
    ///
    /// A relative `path` is taken from `cwd`.
    pub fn covers(&self, cwd: &Path, path: &Path) -> bool {
        let Ok(resolved) = cwd.join(path).canonicalize() else {
            return false;
        };
        self.paths.iter().any(|(attached, folder)| {
            resolved == *attached || (*folder && resolved.starts_with(attached))
        })
    }

    /// Allows `request` when it only reads what was attached.
    ///
    /// Returns `None` for anything else: a request of another kind, one that
    /// names no path, one that names any path not attached, or one whose only
    /// allowing option is "always", which would reach past the attachment.
    pub fn allow_read(
        &self,
        cwd: &Path,
        request: &RequestPermissionRequest,
    ) -> Option<RequestPermissionResponse> {
        if self.paths.is_empty() {
            return None;
        }
        let fields = &request.tool_call.fields;
        if fields.kind != Some(ToolKind::Read) {
            return None;
        }
        let mut named: Vec<&Path> = fields
            .locations
            .iter()
            .flatten()
            .map(|location| location.path.as_path())
            .collect();
        if let Some(raw) = &fields.raw_input {
            named.extend(raw_paths(raw)?);
        }
        if named.is_empty() || !named.iter().all(|path| self.covers(cwd, path)) {
            return None;
        }
        request
            .options
            .iter()
            .any(|option| option.kind == PermissionOptionKind::AllowOnce)
            .then(|| {
                tracing::info!(
                    tool_call = ?request.tool_call.tool_call_id,
                    "allowing a read of an attached file"
                );
                dialog::answer(request, true)
            })
    }
}

/// The paths in a tool call's `rawInput`: every value under a key with
/// "path" in its name, top level only. `None` when such a value is not a
/// string or a list of strings, so the request cannot be vouched for.
fn raw_paths(raw: &serde_json::Value) -> Option<Vec<&Path>> {
    let Some(object) = raw.as_object() else {
        return Some(Vec::new());
    };
    let mut paths = Vec::new();
    for (key, value) in object {
        if !key.to_ascii_lowercase().contains("path") {
            continue;
        }
        match value {
            serde_json::Value::String(path) => paths.push(Path::new(path.as_str())),
            serde_json::Value::Array(items) => {
                for item in items {
                    paths.push(Path::new(item.as_str()?));
                }
            }
            serde_json::Value::Null => {}
            _ => return None,
        }
    }
    Some(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{
        PermissionOption, RequestPermissionOutcome, ToolCallLocation, ToolCallUpdate,
        ToolCallUpdateFields,
    };

    fn attachment(path: &Path) -> Attachment {
        Attachment {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            uri: crate::uri::from_path(path),
        }
    }

    fn options() -> Vec<PermissionOption> {
        vec![
            PermissionOption::new("always", "Always Allow", PermissionOptionKind::AllowAlways),
            PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
            PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
        ]
    }

    /// A request of `kind` naming `paths` as locations and `raw` as input.
    fn request(
        kind: ToolKind,
        paths: &[&Path],
        raw: Option<serde_json::Value>,
        options: Vec<PermissionOption>,
    ) -> RequestPermissionRequest {
        let mut fields = ToolCallUpdateFields::new();
        fields.kind = Some(kind);
        fields.locations = Some(
            paths
                .iter()
                .map(|path| ToolCallLocation::new(*path))
                .collect(),
        );
        fields.raw_input = raw;
        RequestPermissionRequest::new("s", ToolCallUpdate::new("call", fields), options)
    }

    fn read(path: &Path) -> RequestPermissionRequest {
        let raw = serde_json::json!({ "file_path": path });
        request(ToolKind::Read, &[path], Some(raw), options())
    }

    fn chosen(response: Option<RequestPermissionResponse>) -> Option<String> {
        match response?.outcome {
            RequestPermissionOutcome::Selected(selected) => Some(selected.option_id.to_string()),
            _ => None,
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        attached: Attached,
    }

    /// A folder with `notes.md`, `other.md` and `shared/inner.md`, where
    /// `notes.md` and `shared` are attached.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        for name in ["notes.md", "other.md", "shared/inner.md"] {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "x").unwrap();
        }
        let mut attached = Attached::default();
        attached.add(&[
            attachment(&dir.path().join("notes.md")),
            attachment(&dir.path().join("shared")),
        ]);
        Fixture { dir, attached }
    }

    #[test]
    fn reading_an_attached_file_is_allowed_once() {
        let f = fixture();
        let answer = f
            .attached
            .allow_read(f.dir.path(), &read(&f.dir.path().join("notes.md")));
        assert_eq!(chosen(answer).as_deref(), Some("allow"), "once, not always");
    }

    #[test]
    fn reading_inside_an_attached_folder_is_allowed() {
        let f = fixture();
        let inner = f.dir.path().join("shared/inner.md");
        assert!(f.attached.allow_read(f.dir.path(), &read(&inner)).is_some());
    }

    #[test]
    fn a_relative_path_is_taken_from_the_folder() {
        let f = fixture();
        let relative = request(ToolKind::Read, &[Path::new("notes.md")], None, options());
        assert!(f.attached.allow_read(f.dir.path(), &relative).is_some());
    }

    #[test]
    fn reading_a_file_next_to_an_attached_one_still_asks() {
        let f = fixture();
        let other = f.dir.path().join("other.md");
        assert!(f.attached.allow_read(f.dir.path(), &read(&other)).is_none());
        let escaping = f.dir.path().join("shared/../other.md");
        assert!(
            f.attached
                .allow_read(f.dir.path(), &read(&escaping))
                .is_none()
        );
    }

    #[test]
    fn a_link_out_of_an_attached_folder_still_asks() {
        let f = fixture();
        let link = f.dir.path().join("shared/link.md");
        std::os::unix::fs::symlink(f.dir.path().join("other.md"), &link).unwrap();
        assert!(f.attached.allow_read(f.dir.path(), &read(&link)).is_none());
    }

    #[test]
    fn writing_an_attached_file_still_asks() {
        let f = fixture();
        let notes = f.dir.path().join("notes.md");
        for kind in [
            ToolKind::Edit,
            ToolKind::Delete,
            ToolKind::Move,
            ToolKind::Execute,
        ] {
            let asked = request(kind, &[&notes], None, options());
            assert!(
                f.attached.allow_read(f.dir.path(), &asked).is_none(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_read_that_also_names_another_path_still_asks() {
        let f = fixture();
        let notes = f.dir.path().join("notes.md");
        let other = f.dir.path().join("other.md");
        let both = request(ToolKind::Read, &[&notes, &other], None, options());
        assert!(f.attached.allow_read(f.dir.path(), &both).is_none());
        let raw = serde_json::json!({ "file_path": other });
        let mismatched = request(ToolKind::Read, &[&notes], Some(raw), options());
        assert!(f.attached.allow_read(f.dir.path(), &mismatched).is_none());
    }

    #[test]
    fn a_read_that_names_no_path_still_asks() {
        let f = fixture();
        let nameless = request(ToolKind::Read, &[], None, options());
        assert!(f.attached.allow_read(f.dir.path(), &nameless).is_none());
    }

    #[test]
    fn without_a_once_option_it_still_asks() {
        let f = fixture();
        let notes = f.dir.path().join("notes.md");
        let only_always = vec![PermissionOption::new(
            "always",
            "Always Allow",
            PermissionOptionKind::AllowAlways,
        )];
        let asked = request(ToolKind::Read, &[&notes], None, only_always);
        assert!(f.attached.allow_read(f.dir.path(), &asked).is_none());
    }

    #[test]
    fn nothing_attached_allows_nothing() {
        let f = fixture();
        let empty = Attached::default();
        let notes = f.dir.path().join("notes.md");
        assert!(empty.allow_read(f.dir.path(), &read(&notes)).is_none());
    }

    #[test]
    fn attachments_that_are_not_local_files_are_skipped() {
        let mut attached = Attached::default();
        attached.add(&[
            Attachment {
                name: "site".into(),
                uri: "https://example.com/".into(),
            },
            Attachment {
                name: "gone".into(),
                uri: "file:///no/such/file".into(),
            },
        ]);
        assert!(attached.paths.is_empty());
    }
}
