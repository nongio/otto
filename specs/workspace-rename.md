# Workspace Rename

**Status:** draft
**Related specs:** [workspaces-multi-output.md](./workspaces-multi-output.md),
[workspace-reorder.md](./workspace-reorder.md)

## Summary

A workspace can be given a name by editing its label in place, in the workspace
selector shown by expose. The name replaces the default `Workspace N` label
everywhere the workspace appears, and survives a restart.

## Goals

- A second click on a workspace label opens an editor over that label, with the
  current name pre-filled and fully selected.
- The editor supports caret placement, selection (keyboard and mouse), and the
  usual editing keys.
- Enter keeps the typed name; Escape discards it; clicking elsewhere or losing
  keyboard focus keeps it.
- While editing, no keystroke reaches a window or triggers a compositor
  shortcut.
- An empty name falls back to the default `Workspace N`, where `N` is the
  workspace's own number — the one it was given when it was created, not its
  current place in the strip.
- Names survive a compositor restart.
- Each output's workspaces are named independently.

## Non-Goals

- Renaming from anywhere other than the selector (no protocol, no CLI).
- Multi-line names, rich text, or an input method beyond direct key input.
- Clipboard copy/paste inside the field — the field supports it, but the
  compositor does not yet wire a data device into the editor.
- Naming a workspace by its identity rather than its position: names are stored
  per position, so removing a workspace shifts the names after it. Dragging a
  workspace to a new place in the strip does carry its name with it: the name
  is held by the workspace, and the whole strip's names are rewritten against
  the new positions on drop (see
  [workspace-reorder.md](./workspace-reorder.md)).

## Behavior

**Opening the editor**

- Clicking a workspace label once switches to that workspace, as clicking its
  preview does.
- A second click on the same label within the double-click interval opens the
  editor and does *not* switch workspace again.
- The editor appears in the label's place, so no other element moves. It is
  pre-filled with the name currently shown (a custom name, the fullscreen app
  name, or `Workspace N`) and the whole value is selected: the first character
  typed replaces it.
- The caret blinks while the editor is open.

**Editing**

- Printable keys insert at the caret, replacing the selection if there is one.
- Left/Right move the caret; with Ctrl they move by word; with Shift they extend
  the selection. Home/End go to the ends, and extend with Shift. Ctrl+A selects
  everything.
- Backspace and Delete remove the selection if there is one, otherwise one
  character (one word with Ctrl).
- Clicking inside the field places the caret at the nearest character boundary;
  dragging extends the selection; a double click selects a word and a triple
  click selects everything. Shift-click extends from the caret.
- Text wider than the field scrolls horizontally so the caret stays visible.
- A name is at most 32 characters. Text that does not fit is ellipsized when the
  label is drawn normally.

**Ending the edit**

- Enter keeps the typed name and closes the editor.
- Escape closes the editor and leaves the name as it was.
- A click anywhere outside the field keeps the typed name, closes the editor,
  and then acts on whatever was clicked.
- Losing keyboard focus (expose closing, a window taking focus) keeps the typed
  name.
- After the editor closes, keyboard focus returns to the top window of the
  current workspace, or is cleared when it has none.

**The name itself**

- A non-empty name is shown wherever the workspace appears — the selector and
  expose.
- An empty (or whitespace-only) name clears the custom name: the label falls
  back to the fullscreen app name if the workspace is fullscreen, otherwise to
  `Workspace N` where N is the workspace's position, counting from 1.
- A custom name takes precedence over the fullscreen app name, and is not
  cleared when the workspace leaves fullscreen.
- Names are persisted to the writable config file under `[workspaces] names`,
  keyed by `"<output>:<position>"`, and are restored when workspaces are created
  on that output.

## Constraints & Edge Cases

- While the editor is open, the compositor keyboard path forwards every key to
  it: no shortcut fires, and no client receives the key. VT switching and the
  session lock still take precedence, as they do over every other grab.
- Only one label at a time is editable per output. Opening the editor on another
  label first ends the running edit.
- Workspaces are per output, and each output's selector edits only its own.
- Names are keyed by position, not by workspace identity: adding or removing a
  workspace shifts the names of the workspaces after it. Workspace indices come
  from a counter that does not repeat across restarts, so position is the only
  key that survives one.

## Rationale

- **Second click, not a modifier or a menu**: renaming an in-place label is the
  gesture users already know from file managers, and the label is the only thing
  in the selector that a click had no meaning on beyond switching.
- **The editor replaces the label**: reusing the label's box keeps the row
  geometry fixed, so nothing jumps when an edit starts or ends.
- **Focus loss commits**: a name typed and then abandoned by closing expose is
  almost always wanted; Escape is the explicit way to throw it away.
- **A shared text input component**: the field is
  `otto_kit::components::text_input`, so the lock screen, the greeter, and
  future otto-kit dialogs get the same selection model and drawing instead of
  each growing their own.

## Open Questions

- Should the persisted names follow a workspace when it is moved or removed,
  rather than staying with the position?
- Should copy/cut/paste inside the field be wired to the seat's data device?
