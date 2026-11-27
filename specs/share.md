# Share

**Status:** draft  
**Related specs:** [open-with.md](open-with.md), [file-browser.md](file-browser.md), [context-menus.md](context-menus.md), [file-command-palette.md](file-command-palette.md), [stash.md](stash.md)

## Summary

Share hands the selected files to a destination the user has: LocalSend, KDE
Connect, a mail client, Ask, or anything else they add. Otto does not move the
files itself; it lists the destinations installed and launches the one picked,
the way Open With launches an application.

## Goals

- A **Share** menu in Files and on the Stash card, listing the destinations that
  can take what is selected.
- A destination is a desktop entry, so adding one is dropping a file in a
  folder and removing one is deleting it, with no Otto code involved.
- Destinations people commonly have, LocalSend first, work out of the box when
  their application is installed, whether natively or from Flathub.
- A sandboxed destination receives files it could not otherwise read.

## Non-Goals

- Transferring files. Otto does not speak LocalSend, KDE Connect or any other
  protocol; the destination's own application does, with its own device list,
  progress and errors.
- Sharing from other applications. A share portal or bus interface any app can
  call is a later step (see Open Questions).
- Sharing text, links or images on the clipboard. Files only, for now.
- Choosing the destination on the user's behalf, or remembering one.

## Behavior

### Destinations

- A **share destination** is a desktop entry (`Type=Application`) in a
  `otto/share-targets/` folder of the XDG data directories:
  `$XDG_DATA_HOME/otto/share-targets/` (`~/.local/share/otto/share-targets/`)
  first, then each of `$XDG_DATA_DIRS` in order. Otto ships its own in the
  system data directory.
- Entries are identified by file name. One in the user's folder replaces the
  system one of the same name; a user copy with `Hidden=true` removes it.
- An entry is offered only when it is valid and runnable: it has a `Name=` and
  an `Exec=` that takes files (`%f` or `%F`), its `TryExec=` (when present)
  is installed, and `OnlyShowIn`/`NotShowIn` allow Otto. An entry carrying
  `X-Flatpak=⟨app id⟩`, the key Flatpak itself writes into the entries it
  exports, is runnable only while that application is installed, for the user
  or system-wide. An entry that is not runnable is not offered and is not an
  error.
- `MimeType=` says what a destination takes. A destination is offered for a
  selection only when every selected item's type is one it lists or descends
  from one it lists, judged as Open With judges types. A folder's type is
  `inode/directory`, so a destination takes folders only when it lists that.
  `application/octet-stream` takes every file. An entry with no `MimeType=`
  takes every file but no folders.
- `Name=`, `GenericName=` and `Icon=` are what the menu shows, localized as
  every other desktop entry is.
- `X-Otto-Share-Group=` names one destination reached in several ways (the
  same application installed natively and from Flathub). Of the runnable
  entries in a group only one is offered: the first by file name. An entry
  with no group is a group of its own.

### The menu

- In Files, **Share** is a submenu of the context menu whenever the selection
  is one or more items and at least one destination takes them all. It sits
  under Open With. With no destination for the selection it is not shown.
- Each destination is a row with its icon and name, in alphabetical order, with
  the one used last at the top.
- Picking a row launches the destination with the selected items and closes
  the menu. Nothing else happens in Files: the selection, the window and the
  focus stay as they were until the destination's own window appears.
- The palette offers **Share with ⟨destination⟩**, one command per destination
  that takes the selection, under the same condition.
- Share is not offered in the Trash, nor in the file picker.
- On the Stash card, a **Share** button sits beside **Send to Ask** whenever the
  stash holds files and a destination takes them all. It opens the same list as
  a menu. Picking a destination hands it the stashed files, then clears the
  stash as Send to Ask does. Text and screen regions in the stash are not
  shared; while any are stashed, the button's menu says how many items it will
  leave out.

### Launching

- A destination is started from its `Exec=` line with the items' paths in
  place of `%f`/`%F`, exactly as Open With starts an application: other field
  codes expanded, `Path=` and `Terminal=` honoured, detached from Files so that
  closing Files does not close it.
- An `Exec=` taking one file (`%f`) is started once per item.
- A destination that fails to start says so where the action was taken (the
  Files status line, the Stash card) with the destination's name. The selection
  is not lost.
- A sandboxed destination is reached through Flatpak's file forwarding
  (`@@ %F @@`), so the files it receives are readable inside its sandbox
  wherever they are.

### Destinations Otto ships

Each is offered only when its application is installed.

- **LocalSend.** Natively (`localsend_app %F`) or from Flathub (with file
  forwarding), one group, the native copy first. It takes files and folders.
  LocalSend opens on its Send tab with the items already chosen and the device
  is picked there; a copy already running receives them instead of a second
  window opening.

Further destinations are shipped once each is checked against its
application: the command line that hands it files, and what it does with
several. Candidates are KDE Connect, the default mail client through
`xdg-email`, and LocalSend's own command line, `localsend-cli`.

## Constraints & Edge Cases

- Desktop entries in `otto/share-targets/` are not applications: the launcher,
  the dock and Open With never list them, and `update-desktop-database` never
  sees them. The same application may be both an application and a destination.
- One application installed both natively and from Flathub is two entries in
  one group, so the menu shows it once. A user's own entry joins a shipped
  group by naming it, and replaces it by taking the shipped entry's file name.
- A path with no local file (a remote mount Files shows) is handed over as its
  local path when the mount has one, and the destination is not offered when
  it has none.
- The list of destinations is read when the menu opens, so a destination
  installed while Files runs appears the next time.
- Many selected items are handed over in one launch when the entry takes `%F`.
  Very large selections are bounded only by the system's argument length; past
  that the launch fails as any failed start does.

## Rationale

- **No standard exists, so Otto uses the nearest one.** freedesktop has no
  share specification and xdg-desktop-portal no share portal. KDE's Purpose
  framework is a set of Qt plugins loaded into the sharing app, and GNOME's
  `nautilus-sendto` is gone. The Desktop Entry specification is what every
  Linux application already ships and every desktop already launches, so a
  destination built on it costs an application, or a user, one small file, and
  gets field codes, `TryExec`, localisation and Flatpak forwarding for free.
- **A folder of its own, not `MimeType=` on applications.** An application
  that opens a type is not a place to send it: every image viewer would become
  a share destination. Keeping destinations in `otto/share-targets/` keeps the
  two lists apart while keeping the format the same.
- **Otto launches, the destination transfers.** Implementing LocalSend's
  protocol in Otto would duplicate LocalSend, compete with it for its port, and
  leave every other destination out. Handing files to the user's own
  applications keeps one way to share and lets people use what they prefer.
- **One key of Otto's own.** `X-Otto-Share-Group=` is the only addition to
  the Desktop Entry format: without it a native and a Flatpak copy of the same
  application would be listed twice, and `TryExec=` alone cannot say "this or
  that". `X-` keys are how the specification allows extensions.
- **`inode/directory` for folders.** It is the type the shared MIME database
  already gives a folder, so no new key is needed to say "takes folders".

## Open Questions

- Sharing from any application: an `org.otto.Share1` bus method, or backing a
  share portal should one be standardised, so a browser or an image viewer can
  open the same list.
- Sharing text and links (LocalSend takes `--text`): a second `Exec` for text,
  as a `[Desktop Action]`, or text written to a file first.
- A Share pane in Settings listing every destination with a switch, writing the
  `Hidden=true` copies itself.
- Screenshots: offering Share right after one is taken.
