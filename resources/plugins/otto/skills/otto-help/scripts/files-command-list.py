"""The commands one Files script describes, a line each, for `files-command commands`.

Reads the script's `describe` reply on standard input; the script's name is
the argument. Each line is what `files-command run` needs: the script, the
command's id, what it does, what it takes, and the text it asks for.
"""

import json
import sys


def english(text):
    if isinstance(text, dict):
        return text.get("en") or next(iter(text.values()), "")
    return text or ""


def main():
    script = sys.argv[1]
    try:
        commands = json.load(sys.stdin).get("commands", [])
    except (json.JSONDecodeError, AttributeError):
        return
    for command in commands:
        when = command.get("when") or {}
        takes = when.get("targets", "some")
        if when.get("extensions"):
            takes += " of " + ",".join(when["extensions"])
        elif when.get("kinds") and when["kinds"] != "any":
            takes += " " + when["kinds"]
        line = f"{script} {command.get('id')}: {english(command.get('title'))} [files: {takes}]"
        arg = command.get("arg")
        if arg:
            line += f" [--arg: {english(arg.get('prompt'))}"
            choices = arg.get("choices")
            if choices:
                line += ", one of " + ", ".join(
                    f"{choice['value']} ({english(choice.get('title'))})" for choice in choices
                )
            if arg.get("initial"):
                line += f"; default {arg['initial']}"
            line += "]"
        print(line)


main()
