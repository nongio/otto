#!/usr/bin/env python3
"""Exercise Otto's `org.freedesktop.a11y.KeyboardMonitor` without a screen reader.

A screen reader's grabs live and die with its D-Bus connection, so a one-shot
`busctl call` proves nothing: the grab is dropped the moment the tool exits.
This stays connected and prints every `KeyEvent` Otto sends it.

    scripts/a11y-keygrab-test.py               # watch every key, take none
    scripts/a11y-keygrab-test.py --grab F9     # also take F9 from the session
    scripts/a11y-keygrab-test.py --grab-modifier Insert
    scripts/a11y-keygrab-test.py --grab-all    # take everything (Ctrl+C to stop)

With --grab, the grabbed key must stop reaching whatever is focused: press F9
in a text editor and nothing should be typed, while a line appears here.
"""

import argparse
import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

BUS_NAME = "org.freedesktop.a11y.Manager"
OBJECT_PATH = "/org/freedesktop/a11y/Manager"
INTERFACE = "org.freedesktop.a11y.KeyboardMonitor"

# The handful of keysyms worth naming on a command line. Anything else can be
# given as a number: --grab 0xFFC6.
KEYSYMS = {
    "F1": 0xFFBE, "F2": 0xFFBF, "F3": 0xFFC0, "F4": 0xFFC1,
    "F5": 0xFFC2, "F6": 0xFFC3, "F7": 0xFFC4, "F8": 0xFFC5,
    "F9": 0xFFC6, "F10": 0xFFC7, "F11": 0xFFC8, "F12": 0xFFC9,
    "Insert": 0xFF63, "Pause": 0xFF13, "Menu": 0xFF67,
    "Caps_Lock": 0xFFE5, "Num_Lock": 0xFF7F,
}


def keysym(name):
    if name in KEYSYMS:
        return KEYSYMS[name]
    try:
        return int(name, 0)
    except ValueError:
        sys.exit(f"unknown key {name!r} — name one of {', '.join(KEYSYMS)} or give a keysym number")


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--grab", action="append", default=[], metavar="KEY",
                        help="take this key from the session (repeatable)")
    parser.add_argument("--grab-modifier", action="append", default=[], metavar="KEY",
                        help="take this key and anything pressed while it is held")
    parser.add_argument("--grab-all", action="store_true",
                        help="take every key, as a screen reader does in learn mode")
    parser.add_argument("--modifiers", type=lambda v: int(v, 0), default=0, metavar="MASK",
                        help="XKB modifier mask the --grab keys must be pressed under (default 0)")
    args = parser.parse_args()

    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)

    owner = bus.call_sync(
        "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        "NameHasOwner", GLib.Variant("(s)", (BUS_NAME,)),
        GLib.VariantType("(b)"), Gio.DBusCallFlags.NONE, -1, None).unpack()[0]
    if not owner:
        sys.exit(f"{BUS_NAME} is not owned — Otto is not running this session, is "
                 f"nested (--winit), or accessibility.enabled is false")

    proxy = Gio.DBusProxy.new_sync(
        bus, Gio.DBusProxyFlags.DO_NOT_LOAD_PROPERTIES, None,
        BUS_NAME, OBJECT_PATH, INTERFACE, None)

    def on_signal(_proxy, _sender, signal, params):
        if signal != "KeyEvent":
            return
        released, state, sym, unichar, code = params.unpack()
        line = (f"{'release' if released else 'press':<7} "
                f"keysym=0x{sym:04X} mods=0x{state:02X} keycode={code}")
        if unichar:
            line += f" char={chr(unichar)!r}"
        print(line, flush=True)

    proxy.connect("g-signal", on_signal)

    proxy.call_sync("WatchKeyboard", None, Gio.DBusCallFlags.NONE, -1, None)
    what = ["watching every key"]

    if args.grab_all:
        proxy.call_sync("GrabKeyboard", None, Gio.DBusCallFlags.NONE, -1, None)
        what.append("grabbing every key")

    if args.grab or args.grab_modifier:
        modifiers = [keysym(k) for k in args.grab_modifier]
        keystrokes = [(keysym(k), args.modifiers) for k in args.grab]
        proxy.call_sync("SetKeyGrabs",
                        GLib.Variant("(aua(uu))", (modifiers, keystrokes)),
                        Gio.DBusCallFlags.NONE, -1, None)
        if args.grab:
            what.append(f"grabbing {', '.join(args.grab)}")
        if args.grab_modifier:
            what.append(f"grabbing modifier {', '.join(args.grab_modifier)}")

    print(f"{'; '.join(what)}. Ctrl+C to stop — grabs end with this connection.",
          flush=True)

    try:
        GLib.MainLoop().run()
    except KeyboardInterrupt:
        print("\nstopped")


if __name__ == "__main__":
    main()
