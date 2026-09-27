#!/usr/bin/env python3
"""Drive the gram TUI in a pty and dump the rendered screen.

Usage:  python3 tools/drive_tui.py "<keys>" [cols] [rows]

Keys are sent literally; use \\n, \\t, \\e for Enter, Tab and Escape.
Rendering is replayed through a tiny ANSI interpreter so the output is
what a terminal would actually show.
"""

import os
import pty
import re
import select
import sys
import time

COLS, ROWS = 100, 30


class Screen:
    """Just enough of a terminal to check a TUI's layout."""

    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.clear()

    def clear(self):
        self.grid = [[" "] * self.cols for _ in range(self.rows)]
        self.x = self.y = 0

    def put(self, ch):
        if self.y < self.rows and self.x < self.cols:
            self.grid[self.y][self.x] = ch
        self.x += 1

    def feed(self, data):
        i = 0
        while i < len(data):
            ch = data[i]
            if ch == "\x1b":
                m = re.match(r"\x1b\[([0-9;?]*)([A-Za-z])", data[i:])
                if m:
                    self.csi(m.group(1), m.group(2))
                    i += m.end()
                    continue
                m = re.match(r"\x1b[\]P].*?(\x07|\x1b\\)", data[i:], re.S)
                if m:
                    i += m.end()
                    continue
                i += 2
                continue
            if ch == "\r":
                self.x = 0
            elif ch == "\n":
                self.y += 1
                self.x = 0
            elif ch == "\b":
                self.x = max(0, self.x - 1)
            elif ch >= " ":
                self.put(ch)
            i += 1

    def csi(self, params, final):
        args = [int(p) for p in params.split(";") if p.isdigit()]

        def arg(n, default=1):
            return args[n] if len(args) > n else default

        if final == "H":
            self.y = max(0, arg(0) - 1)
            self.x = max(0, arg(1) - 1)
        elif final == "J" and arg(0, 0) in (2, 3):
            self.clear()
        elif final == "K":
            for x in range(self.x, self.cols):
                self.grid[self.y][x] = " "
        elif final == "A":
            self.y = max(0, self.y - arg(0))
        elif final == "B":
            self.y = min(self.rows - 1, self.y + arg(0))
        elif final == "C":
            self.x = min(self.cols - 1, self.x + arg(0))
        elif final == "D":
            self.x = max(0, self.x - arg(0))

    def render(self):
        return "\n".join("".join(row).rstrip() for row in self.grid)


def main():
    keys = sys.argv[1] if len(sys.argv) > 1 else ""
    keys = keys.replace("\\n", "\r").replace("\\t", "\t").replace("\\e", "\x1b")
    cols = int(sys.argv[2]) if len(sys.argv) > 2 else COLS
    rows = int(sys.argv[3]) if len(sys.argv) > 3 else ROWS

    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.execvp("./target/debug/gram", ["gram"])

    import fcntl
    import struct
    import termios

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    screen = Screen(cols, rows)

    def drain(timeout=0.35):
        deadline = time.time() + timeout
        while time.time() < deadline:
            ready, _, _ = select.select([fd], [], [], 0.05)
            if not ready:
                continue
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return False
            if not chunk:
                return False
            screen.feed(chunk.decode("utf-8", "replace"))
            deadline = time.time() + 0.15
        return True

    drain(0.8)
    for key in keys:
        os.write(fd, key.encode())
        time.sleep(0.06)
        drain(0.3)

    print(screen.render())

    os.write(fd, b"\x1b")
    time.sleep(0.05)
    os.write(fd, b":q!\r")
    time.sleep(0.2)
    try:
        os.close(fd)
    except OSError:
        pass
    os.waitpid(pid, os.WNOHANG)


if __name__ == "__main__":
    main()
