#!/usr/bin/env python3
"""Every translated catalogue carries every key the source catalogue does.

The same check as otto-kit's `translated_locales_are_complete` test, without
compiling otto-kit, so CI can run it in seconds before the Rust jobs. Keys
are read the way `otto_kit::i18n::keys_in` reads them: a message starts at
column zero with `key =`; indented lines, comments (`#`) and terms (`-key`)
are not keys. `en-US` is a sparse overlay and exempt.
"""

import sys
from pathlib import Path

LOCALES = Path(__file__).resolve().parent.parent / "resources" / "locales"
SOURCE = "en-GB"
EXEMPT = {"en-US"}


def keys_in(text: str) -> set[str]:
    keys = set()
    for line in text.splitlines():
        if not line or line[0].isspace() or line.startswith("#"):
            continue
        key, sep, _ = line.partition("=")
        key = key.strip()
        if sep and key and not key.startswith("-"):
            keys.add(key)
    return keys


def main() -> int:
    source = keys_in((LOCALES / f"{SOURCE}.ftl").read_text(encoding="utf-8"))
    gaps = []
    for path in sorted(LOCALES.glob("*.ftl")):
        locale = path.stem
        if locale == SOURCE or locale in EXEMPT:
            continue
        missing = sorted(source - keys_in(path.read_text(encoding="utf-8")))
        if missing:
            gaps.append(f"{locale}: {', '.join(missing)}")
    if gaps:
        print("keys missing from catalogues:")
        print("\n".join(gaps))
        return 1
    print(f"all catalogues carry every {SOURCE} key ({len(source)} keys)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
