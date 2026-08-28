#!/usr/bin/env python3
"""Read Otto's accessible trees from a terminal, without a screen reader.

Orca is the real test, but it is a lot of machinery to install and listen to
just to find out whether a control is described the way it is drawn. This does
the same reading over AT-SPI and prints it.

    scripts/a11y-watch.py                    # list every accessible application
    scripts/a11y-watch.py Otto               # dump the shell's tree
    scripts/a11y-watch.py otto-settings      # dump an application's tree
    scripts/a11y-watch.py --follow           # print focus as it moves, like Orca speaks it

Merely running this registers as an assistive technology, which is what makes
applications publish their trees at all — GTK, Qt and Electron all stay silent
until one is listening.
"""

import argparse
import sys

import pyatspi

# What a screen reader leads with. Anything not named here is read by role name.
ANNOUNCED_STATES = [
    (pyatspi.STATE_CHECKED, "checked"),
    (pyatspi.STATE_EXPANDED, "expanded"),
    (pyatspi.STATE_SELECTED, "selected"),
]


def describe(node):
    """One line, in the order a screen reader would say it."""
    role = node.getRoleName()
    name = node.name or ""
    parts = [f"{name!r}" if name else "<unnamed>", role]

    states = node.getState()
    # Applications, frames and other containers carry no ENABLED state, so its
    # absence there means nothing — only a control can be disabled.
    controls = role not in {"application", "frame", "window", "panel", "filler"}
    if controls and not states.contains(pyatspi.STATE_ENABLED):
        parts.append("disabled")
    for state, said in ANNOUNCED_STATES:
        if states.contains(state):
            parts.append(said)

    try:
        value = node.queryValue()
        parts.append(f"value {value.currentValue:g} of {value.minimumValue:g}–{value.maximumValue:g}")
    except NotImplementedError:
        pass

    try:
        text = node.queryText()
        contents = text.getText(0, -1)
        if contents and contents != name:
            parts.append(f"text {contents!r}")
    except NotImplementedError:
        pass

    actions = []
    try:
        action = node.queryAction()
        actions = [action.getName(i) for i in range(action.nActions)]
    except NotImplementedError:
        pass
    if actions:
        parts.append(f"[{', '.join(actions)}]")

    return "  ".join(parts)


def dump(node, depth=0, limit=6):
    print("  " * depth + describe(node))
    if depth >= limit:
        return
    for child in node:
        if child is not None:
            dump(child, depth + 1, limit)


def applications():
    return [app for app in pyatspi.Registry.getDesktop(0) if app is not None]


def find(pattern):
    matches = [app for app in applications()
               if pattern.lower() in (app.name or "").lower()]
    if not matches:
        names = ", ".join(app.name or "<unnamed>" for app in applications()) or "nothing"
        sys.exit(f"no accessible application matching {pattern!r}. On the bus: {names}")
    return matches


def follow():
    """Print what has focus as it changes — the thing Orca would speak."""

    def on_focus(event):
        if event.type.startswith("object:state-changed") and not event.detail1:
            return
        try:
            source = event.source
            app = source.getApplication()
            where = (app.name if app else "?") or "?"
            print(f"[{where}] {describe(source)}", flush=True)
        except Exception as err:  # a node can vanish between event and read
            print(f"<gone: {err}>", flush=True)

    for name in ("object:state-changed:focused", "object:selection-changed",
                 "object:text-caret-moved", "window:activate"):
        pyatspi.Registry.registerEventListener(on_focus, name)

    print("following focus — move around with Tab, the app switcher, the dock. "
          "Ctrl+C to stop.", flush=True)
    try:
        pyatspi.Registry.start()
    except KeyboardInterrupt:
        pyatspi.Registry.stop()


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("app", nargs="?", help="application name, or part of one")
    parser.add_argument("--follow", action="store_true",
                        help="print focus changes as they happen instead of dumping a tree")
    parser.add_argument("--depth", type=int, default=6, help="how deep to dump (default 6)")
    args = parser.parse_args()

    if args.follow:
        follow()
        return

    if not args.app:
        apps = applications()
        if not apps:
            sys.exit("nothing is on the accessibility bus")
        print(f"{len(apps)} accessible application(s):")
        for app in apps:
            print(f"  {app.name or '<unnamed>'}  ({app.childCount} window(s))")
        print("\nPass a name to dump one — e.g. `a11y-watch.py Otto`.")
        return

    for app in find(args.app):
        dump(app, limit=args.depth)


if __name__ == "__main__":
    main()
