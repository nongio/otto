# 0018: Renaming Preview

**Status:** Draft

## Goal

Give the app in [0016](0016-agent-edited-documents.md) a name that says what it
becomes: the place where you open a document and change it by asking. The
preference is **Studio**. The rename ships with the chat sidebar, not before.

## Why rename

- **"Preview" means "look, don't touch".** Once you edit through the sidebar,
  with versions and undo, the name hides the main thing the app does.
- **It overlaps Peek.** Peek is the preview over the file list, and
  `specs/preview-app.md` describes Preview as Peek in its own window. Two names
  for "glance at it" is one too many.
- **It invites the macOS comparison.** "Otto's Preview" is read against Apple's
  Preview, a comparison we don't choose to make.

## Options

| Name | For | Against |
|---|---|---|
| **Studio** (preferred) | Says you make things here; names the reason the app exists (0016). Short, a plain noun like Files and Settings | Over-promises with no agent set up; a crowded word (Android Studio, OBS Studio) |
| Viewer | The plainest; honest for everyone who just opens a file | Says nothing about editing; too generic to search for |
| Preview (keep) | Known, honest today, no migration | The three problems above |
| Proof | A copy you review and mark up before signing off: fits marks and versions | Needs explaining when you open a JPEG; other meanings |
| Loupe | Looking closely, pointing | GNOME's image viewer is already called Loupe |
| No name | The window is the file; menus say "Open with Otto" | Nothing to call it in docs, support and settings |

Rejected along the way: Draft (sounds like a text editor), Lightbox (photo-only,
a web overlay), Desk (too close to "desktop"), Darkroom (photo-only, darktable's
word), names about the agent such as Ask (wrong for anyone without one).

## Why Studio

Most people will open a file and close it, and some will never set up an agent.
Viewer serves them best, but names the app after what it shares with every other
viewer. 0016 makes editing the point of the app, so the name follows the point.
Studio's weak spots are handled by the app, not the name:

- **No agent set up:** the app is still a fast, many-format viewer with versions.
  The sidebar button stays, and offers to set an agent up instead of hiding.
- **Too grand for a JPEG:** menus, search and Open With show a plain
  `GenericName` and `Comment` ("Viewer", "View and edit photos, PDFs and
  documents"), so nobody has to guess.
- **A crowded word:** inside Otto it sits next to Files and Settings, and the
  binary is `otto-studio`. Search finds it by its keywords.

Final wording, translations included, goes through otto-copywriter.

## When

With 0016's milestone 3, the chat sidebar. Renaming earlier promises editing the
app doesn't have; renaming with the sidebar makes the new name the announcement.

## What changes

| Where | From | To |
|---|---|---|
| Crate and folder | `components/otto-preview` | `components/otto-studio` |
| Binary, `TryExec`, `StartupWMClass` | `otto-preview` | `otto-studio` |
| Desktop file | `resources/otto-preview.desktop` | `resources/otto-studio.desktop` |
| `Name` and its 10 translations | Preview, Vorschau, Aperçu, … | Studio, translated or kept per locale |
| `GenericName`, `Comment`, `Keywords` | File Viewer, "View …" | Viewer, "View and edit …"; keywords keep `preview` and `viewer` |
| D-Bus | `org.otto.Preview1`, `/org/otto/Preview1` | `org.otto.Studio1`, `/org/otto/Studio1` |
| Gateway tool list (0014) | `preview.toml`, `preview_*` tools | `studio.toml`, `studio_*` tools |
| Session `_meta.otto.app` (0016) | `otto-preview` | `otto-studio` |
| Workspace members and packaging | `Cargo.toml` (members and the three package asset lists), `PKGBUILD`, `PKGBUILD-git`, `PKGBUILD-nightly-bin`, `scripts/packaging/make-arch-tarball.sh`, `scripts/packaging/verify-install.sh`, `.github/workflows/ci.yml` | `otto-studio` |
| Spec | `specs/preview-app.md` | `specs/studio.md`, rewritten for 0016 (its non-goals change) |
| Docs and plans | 0016, 0017, user docs | Studio |

Not renamed: Peek, the preview pane in Files, `otto-kit`'s `preview` module and
the settings and theme previews. Those are previews.

If the rename lands before 0016's tools exist (likely), the `preview_*` tools
and `preview.toml` are born with the new name and nothing needs migrating.

## Migration

- **Default apps.** People who set Preview as the default for PDFs or pictures
  have `otto-preview.desktop` in `mimeapps.list`. For one release, ship
  `otto-preview.desktop` as `NoDisplay=true` with `Exec=otto-studio %f`, so
  those defaults keep working and nothing shows twice in menus.
- **Old command.** `otto-preview` stays for one release as a symlink to
  `otto-studio`, for scripts and keybindings.
- **D-Bus.** One release where the app owns both names. Nothing outside Otto
  calls it yet, so this is short.
- **Release notes** say it once: "Preview is now Studio. It still opens
  everything; now you can ask it to edit."

## Milestones

1. **Decide.** Settle the name and the `GenericName`, `Comment` and translations
   with otto-copywriter. Update 0016.
2. **Rename** in one PR: folder, crate, binary, desktop file, D-Bus name, spec,
   packaging, CI, docs. No behaviour change.
3. **Compatibility shims:** the hidden old desktop file and the symlink, both
   marked for removal.
4. **Remove the shims** one release later.

## Testing

- `scripts/packaging/verify-install.sh` lists `otto-studio` and the hidden old
  desktop file.
- A user with `otto-preview.desktop` as their PDF default opens a PDF from Files:
  Studio opens it.
- Running `otto-preview <file>` opens Studio, once, as the single instance.
- `desktop-file-validate` on both desktop files.

## Open questions

- **Translations of the name.** Translate "Studio" per locale (it is a common
  loanword in most of Otto's locales), or keep it as a product name everywhere?
- **The icon.** `image-viewer` today. Studio wants its own.
- **Before the sidebar.** If the sidebar slips, does the overlap with Peek justify
  renaming early to Viewer and again later? Probably not: one rename only.
