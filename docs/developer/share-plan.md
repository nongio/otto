# Share — implementation plan

How [Share](../../specs/share.md) gets built: what exists to build on, where
each piece goes, and in what order.

> **Status: not built.** This is the plan. The spec is the contract; this
> document is only how we intend to meet it.

## The idea in one paragraph

Linux has no share standard: no freedesktop spec, no share portal, KDE's
Purpose is in-process Qt plugins, and GNOME's `nautilus-sendto` is gone. So
Otto does not invent a protocol or reimplement LocalSend. A **share
destination is a desktop entry** in `otto/share-targets/` under the XDG data
dirs. Files and Stash list the ones that take the selection and launch the one
picked, through the same launch code Open With uses. LocalSend is the first
destination we ship; anything else is a file the user drops in.

```
Files context menu / palette ─┐
                              ├─► otto_kit::share ──► otto_kit::mime_apps::launch
Stash card "Share" button ────┘        │                    (Exec, %F, Path,
                                       │                     Terminal, Flatpak @@)
              ~/.local/share/otto/share-targets/*.desktop
              /usr/share/otto/share-targets/*.desktop
```

## What already exists

- **`otto_kit::mime_apps`** (`components/otto-kit/src/mime_apps.rs`): reads
  desktop entries, the MIME association rules and type ancestry, and
  `open(app, paths)` launches an entry with field codes expanded,
  `Path=`/`Terminal=` honoured and the child detached. Share launches through
  it, not through a copy.
- **Type detection and ancestry**, the way Open With judges a selection
  (`components/otto-files/src/open_with.rs`), so "takes every selected item" is
  answered the same way "can open every selected file" is.
- **The Files context menu and palette** (`components/otto-files/src/command.rs`,
  where `OPEN_WITH` is defined), which already has submenus.
- **The Stash card** (`components/otto-stash/src/balloon.rs`) and its
  **Send to Ask** button.
- **Packaging**: `Cargo.toml`'s `cargo-deb` and `cargo-generate-rpm` asset
  lists already install files under `usr/share/otto/`.

## Phase 1: destinations as data (`otto-kit`)

A new module, `components/otto-kit/src/share.rs`, next to `mime_apps`. It is
pure lookup, no UI:

- `Destinations::load()` scans `$XDG_DATA_HOME/otto/share-targets/` then each
  `$XDG_DATA_DIRS` entry, first file name wins, `Hidden=true` removes.
- Each entry becomes the same `mime_apps::App` Open With uses, plus its
  `X-Otto-Share-Group=`. Parsing goes through the existing desktop-entry
  reader; no second parser.
- Runnable check: the `TryExec=` and `OnlyShowIn`/`NotShowIn` filtering
  `mime_apps` already applies to applications (factored out so both use it),
  plus `Exec=` having `%f`/`%F` and `X-Flatpak=` resolved against `~/.local/share/flatpak/app/<id>` and
  `/var/lib/flatpak/app/<id>`.
- `for_items(&[(path, ancestors)]) -> Vec<&App>`: filtered by `MimeType=`
  (absent means every file, no folders; `inode/directory` means folders),
  one per group, sorted by name with the last used first.
- Last used: one line in `$XDG_STATE_HOME/otto/share-last`, written by
  `share::launch`.
- `share::launch(dest, paths)` calls `mime_apps::open`, which already starts
  one process per file for an entry that only takes `%f`.

Unit tests in the module, on temp dirs with a fake `XDG_DATA_*`: precedence,
`Hidden=true`, `TryExec` misses, `MimeType` filtering including folders and
`octet-stream`, groups, ordering.

`otto-kit` is in the kit layer and already holds `mime_apps`, so no
`architecture.toml` change.

## Phase 2: LocalSend ships (`resources/`)

- `resources/share-targets/localsend.desktop`:
  `Exec=localsend_app %F`, `TryExec=localsend_app`,
  `X-Otto-Share-Group=localsend`,
  `MimeType=application/octet-stream;inode/directory;`,
  `Icon=org.localsend.localsend_app`.
- `resources/share-targets/localsend-flatpak.desktop`:
  `Exec=flatpak run --file-forwarding org.localsend.localsend_app @@ %F @@`,
  `X-Flatpak=org.localsend.localsend_app`, same group. It sorts after the
  native one, so a native install wins.
- Asset lines in both packaging tables → `usr/share/otto/share-targets/`.
- **To verify before merging**: LocalSend's argument handling (checked in
  `app/lib/config/init.dart` and `selected_sending_files_provider.dart`: paths
  become the Send tab's selection, folders recursively, and arguments are
  forwarded to an already-running copy), on a real install, both native and
  Flatpak. That the Flatpak build reads `--file-forwarding` paths from the
  document portal is the part not yet tested.

## Phase 3: Share in Files (`otto-files`)

- `command.rs`: an `id::SHARE` submenu under Open With, built from
  `share::Destinations::for_items` for the selection; hidden when empty, in
  the Trash and in the picker.
- Palette: one `Share with ⟨name⟩` command per destination, same condition.
- Failure goes to the status line, like a failed Open With.
- The destinations are loaded when the menu opens, not cached on the window.
- Localisation: new keys in `en-GB.ftl` (the `l10n-translator` agent fills the
  others).

Headless test (`tests/`, `--features headless`): a fake destination entry
whose `Exec` writes its arguments to a file; right-click a selection, pick
Share → destination, assert the file lists the paths.

## Phase 4: Share on the Stash card (`otto-stash`)

- A **Share** button beside **Send to Ask**, shown when the stash holds files
  a destination takes; it opens the destinations as a menu.
- Picking one launches with the stashed files and clears the stash, like
  Send to Ask. The menu notes how many text or region items it leaves out.
- Update `specs/stash.md` in the same change.

## Phase 5: more destinations

Each is shipped only after its command line is checked against the real
application. Candidates: KDE Connect, the default mail client through
`xdg-email --attach` (one launch per file unless it takes several), and
`localsend-cli send` in a terminal for "send without opening the app".

## Later (open questions in the spec)

- `org.otto.Share1.Share(paths)` in `otto-dbus` so any app can open the list,
  and an `org.freedesktop.impl.portal` backend should a share portal ever be
  standardised.
- Sharing text and links.
- A Share pane in Settings: every destination with a switch (writing
  `Hidden=true` copies), and a button to open the folder.
- Share after a screenshot is taken.

## Docs to update as phases land

- `specs/share.md`: status to `stable` once phases 1–4 are in.
- `docs/user/`: a short page on adding a destination of your own, with the
  LocalSend entry as the example.
- `resources/plugins/otto/skills/otto-help/references/`: the same, so Ask can
  answer "how do I share to X".
