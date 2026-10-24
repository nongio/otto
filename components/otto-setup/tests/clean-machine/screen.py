"""Drive otto-setup in a pseudo-terminal and print the screen it ends on.

    screen.py /usr/bin/otto-setup enter tab enter

Each argument is a key: enter, tab, down, up, space, esc, q, or any other
text typed as is. `shot` prints the screen at that point, then goes on.
The last screen is always printed. Lines are what a 100-column terminal
would show, without colours.
"""
import fcntl
import os
import pty
import re
import select
import struct
import sys
import termios
import time

ROWS, COLS = 40, 100
KEYS = {
    "down": b"\x1b[B",
    "up": b"\x1b[A",
    "enter": b"\r",
    "space": b" ",
    "esc": b"\x1b",
    "tab": b"\t",
}

binary, keys = sys.argv[1], sys.argv[2:]
pid, fd = pty.fork()
if pid == 0:
    os.execv(binary, [binary])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))

buf = b""


def pump(seconds):
    global buf
    end = time.time() + seconds
    while time.time() < end:
        ready, _, _ = select.select([fd], [], [], 0.05)
        if ready:
            try:
                buf += os.read(fd, 65536)
            except OSError:
                return


def screen():
    text = buf.decode("utf-8", "replace")
    rows = {}
    for m in re.finditer(r"\x1b\[(\d+);1H(.*?)(?=\x1b\[\d+;1H|$)", text, re.S):
        line = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", m.group(2))
        rows[int(m.group(1))] = line.rstrip()
    lines = [rows.get(i, "") for i in range(1, ROWS + 1)]
    while lines and not lines[-1]:
        lines.pop()
    return "\n".join(lines)


pump(1.5)
for key in keys:
    if key == "shot":
        print(screen())
        print("=" * COLS)
        continue
    buf = b""
    os.write(fd, KEYS.get(key, key.encode()))
    pump(0.5)
print(screen())
try:
    os.kill(pid, 9)
except ProcessLookupError:
    pass
