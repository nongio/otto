"""The request Files would send a script's `run`, for `files-command run`.

Arguments: the command's id, "1" when an argument was given, the argument,
then the files. The files are selected in the folder the first one is in.
"""

import json
import os
import sys


def main():
    command, has_arg, arg, *files = sys.argv[1:]
    targets = [os.path.abspath(os.path.expanduser(path)) for path in files]
    for path in targets:
        if not os.path.exists(path):
            sys.exit(f"files-command: no such file: {path}")
    folder = os.path.dirname(targets[0])
    request = {
        "command": command,
        "locale": "en-GB",
        "targets": targets,
        "situation": {
            "path": folder,
            "selection": targets,
            "siblings": sorted(os.listdir(folder)),
        },
    }
    if has_arg:
        request["arg"] = arg
    print(json.dumps(request))


main()
