#!/usr/bin/env python3
"""Render the TUI at several sizes and insist the layout holds.

A width bug does not crash gram — it quietly pushes a border past the right
edge, which only a human staring at a terminal would notice. Korean terms are
the usual trigger: 한 is two columns wide, so code that counts characters
where it should count columns is off by one per syllable.

Usage:  python3 tools/check_layout.py
Builds its own scratch collection, so it never touches yours. Uses $GRAM_BIN,
or target/release/gram, or target/debug/gram.
"""

import os
import shutil
import subprocess
import sys
import tempfile
import unicodedata

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

DECK = "한국어"
# One card, so which card a screen shows is never in question.
FIXTURE = (
    "term,pron,pos,definition,example\n"
    "한국어,han guk eo,n,the Korean language,나는 한국어를 공부한다.\n"
)

# Keys to send, and what must survive the rendering, at every size below.
SCREENS = [
    ("", ["DECK", DECK]),  # the deck list
    ("s", [DECK]),  # a review card, question side
    ("s ", [DECK, "the Korean language"]),  # revealed
    ("\\n", [DECK, "STATE"]),  # the browser
    ("?", ["MOVING AROUND"]),  # help
    (":restore\\n", ["no kept copies"]),  # a status-line message
]

# 60x20 is about the smallest usable terminal; 100x30 the comfortable case.
SIZES = [(60, 20), (80, 24), (100, 30)]


def columns(text):
    """Display columns `text` occupies: 한 is two, a combining accent none."""
    width = 0
    for ch in text:
        if unicodedata.combining(ch):
            continue
        width += 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1
    return width


def binary():
    if os.environ.get("GRAM_BIN"):
        return os.environ["GRAM_BIN"]
    for build in ("release", "debug"):
        path = os.path.join(ROOT, "target", build, "gram")
        if os.path.exists(path):
            return path
    raise SystemExit("no gram binary — cargo build first, or set $GRAM_BIN")


def render(keys, cols, rows, env):
    out = subprocess.run(
        [sys.executable, os.path.join(HERE, "drive_tui.py"), keys, str(cols), str(rows)],
        capture_output=True,
        text=True,
        timeout=60,
        env=env,
    )
    if out.returncode != 0:
        raise SystemExit(f"drive_tui.py failed at {cols}x{rows}:\n{out.stderr}")
    # Rows come back with trailing blanks stripped, so a line may be shorter
    # than the screen — never longer.
    return out.stdout.rstrip("\n").split("\n")


def main():
    gram = binary()
    work = tempfile.mkdtemp(prefix="gram-layout-")
    try:
        env = dict(os.environ, GRAM_BIN=gram, GRAM_DATA=os.path.join(work, "collection.json"))
        source = os.path.join(work, "deck.csv")
        with open(source, "w", encoding="utf-8") as handle:
            handle.write(FIXTURE)
        made = subprocess.run(
            [gram, "import", source, DECK], capture_output=True, text=True, env=env
        )
        if made.returncode != 0:
            raise SystemExit(f"fixture import failed:\n{made.stderr}")

        failures = []
        for cols, rows in SIZES:
            for keys, expected in SCREENS:
                where = f"{cols}x{rows} keys={keys!r}"
                lines = render(keys, cols, rows, env)
                screen = "\n".join(lines)

                if len(lines) > rows:
                    failures.append(f"{where}: {len(lines)} lines, screen holds {rows}")

                for n, line in enumerate(lines, 1):
                    if columns(line) > cols:
                        failures.append(
                            f"{where}: line {n} overflows to {columns(line)} "
                            f"columns: {line!r}"
                        )
                        break  # one example per screen is enough to debug

                for want in expected:
                    if want not in screen:
                        failures.append(f"{where}: {want!r} is missing")

        if failures:
            print("\n".join(f"FAIL {f}" for f in failures))
            raise SystemExit(1)
        print(f"layout holds at {', '.join(f'{c}x{r}' for c, r in SIZES)}")
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
