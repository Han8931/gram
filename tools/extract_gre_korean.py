#!/usr/bin/env python3
"""Pull the Korean glosses out of sources/gre_vocab.pdf.

The PDF is a spreadsheet print of 거의만점어휘집: two half-pages side by side,
each one a table of HEADWORD | Korean gloss | GRE analogy hint. `pdftotext
-layout` interleaves the halves and puts a wrapped gloss on the line above its
own headword, so the columns are read off word bounding boxes instead.

Usage:  python3 tools/extract_gre_korean.py [out.csv]
Writes term,korean — one row per headword, in the order the book prints them.
Needs pdftotext (poppler).
"""

import csv
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

NS = "{http://www.w3.org/1999/xhtml}"
PDF = "sources/gre_vocab.pdf"

# Where each column of a half-page starts. A half is 297pt wide; the hint
# column runs from the end of the gloss to the edge of the half. The gloss
# column ends where it does because no gloss word starts past 131 on the left
# or 413 on the right, and no hint word starts before 139 or 422 — the columns
# are a spreadsheet's, so the gap between them is the same on every page.
HALVES = [
    {"term": (10.0, 65.0), "gloss": (65.0, 135.0)},
    {"term": (297.0, 345.0), "gloss": (345.0, 417.0)},
]
# The book's running header, which is not a row.
HEADER_BELOW = 30.0
# Gap between two words on a line that means a space rather than a kerned pair.
SPACE_GAP = 1.0
# Room left at the end of a line that means the character after it could have
# fitted, so what the break swallowed was a space. The widest Korean glyph in
# the book is 7.32pt; measuring the next character instead of taking the widest
# possible one splits words that only looked like they had room (`경험적 은`
# for `경험 적은`), and a missing space reads better than a split word.
WRAP_SLACK = 7.5

# A headword is printed in capitals. Not every one is plain ASCII (CLICHÉ,
# NAÏVE) and a few carry an alternative spelling in lower case (`RUMINANT or
# RUMINATIVE`), so the test is that it starts with a capital and holds no
# Korean — the page footer `29 ]` is what it is written to exclude.
HEADWORD = re.compile(r"[A-ZÀ-Þ][^가-힣\]]*$")


def words(pdf):
    """Every word of the PDF as (page, xMin, xMax, yMin, yMax, text)."""
    xml = subprocess.run(
        ["pdftotext", "-bbox-layout", pdf, "-"],
        capture_output=True,
        check=True,
    ).stdout
    root = ET.fromstring(xml)
    out = []
    for number, page in enumerate(root.iter(f"{NS}page")):
        for word in page.iter(f"{NS}word"):
            text = (word.text or "").strip()
            if not text:
                continue
            out.append(
                (
                    number,
                    float(word.get("xMin")),
                    float(word.get("xMax")),
                    float(word.get("yMin")),
                    float(word.get("yMax")),
                    text,
                )
            )
    return out


def join_line(line):
    """Words of one display line, spaced as the print spaces them."""
    text = line[0][5]
    for previous, word in zip(line, line[1:]):
        if word[1] - previous[2] > SPACE_GAP:
            text += " "
        text += word[5]
    return text


def into_lines(words):
    """Words grouped into display lines, top to bottom, each left to right."""
    lines = []
    for word in sorted(words, key=lambda w: (round(w[3], 1), w[1])):
        if lines and abs(word[3] - lines[-1][0][3]) < 2.0:
            lines[-1].append(word)
        else:
            lines.append([word])
    return lines


def centre(line):
    """Where a display line sits vertically."""
    return (min(w[3] for w in line) + max(w[4] for w in line)) / 2


def assign(lines, anchors):
    """Split `lines` into one run per headword in `anchors`.

    A spreadsheet centres both a headword and its gloss in the row, so the run
    of lines belonging to a headword is the one whose block centre sits closest
    to it. Rows of different heights make the midpoint between two headwords
    fall inside the taller one's text, which is why the runs are solved for
    together rather than cut at a boundary.
    """
    spans = [(min(w[3] for w in line), max(w[4] for w in line)) for line in lines]
    best = {(0, 0): (0.0, None)}
    # No row in the book runs to more than five lines.
    for taken, anchor in ((t, a) for a in range(len(anchors)) for t in range(len(lines))):
        if (taken, anchor) not in best:
            continue
        cost, _ = best[(taken, anchor)]
        for size in range(1, 6):
            end = taken + size
            if end > len(lines):
                break
            block = (spans[taken][0] + spans[end - 1][1]) / 2
            total = cost + abs(block - anchors[anchor])
            key = (end, anchor + 1)
            if key not in best or total < best[key][0]:
                best[key] = (total, (taken, anchor))

    key = (len(lines), len(anchors))
    if key not in best:
        return None
    runs = []
    while key != (0, 0):
        previous = best[key][1]
        runs.append(lines[previous[0] : key[0]])
        key = previous
    runs.reverse()
    return runs


def join_cell(lines, right_edge):
    """One cell's display lines as its text.

    Korean wraps mid-word, so a line that filled the column runs straight into
    the next one. Where the line stopped a whole character short of the edge,
    what the break swallowed was a space, and it goes back in. A break that
    fills the column is ambiguous — the swallowed space is nowhere in the file —
    so a gloss can come out with two words run together.
    """
    text = ""
    for number, line in enumerate(lines):
        if number and max(w[2] for w in lines[number - 1]) < right_edge - WRAP_SLACK:
            text += " "
        text += join_line(line)
    return text.strip()


def rows(pdf=PDF):
    """(headword, gloss) for every row the book prints, in its own order."""
    every = words(pdf)
    body = [w for w in every if w[3] > HEADER_BELOW]
    # How far a gloss can reach before the column wraps it, measured over the
    # whole book: a page where no gloss fills the column would read short.
    edges = [
        max(w[2] for w in body if half["gloss"][0] <= w[1] < half["gloss"][1])
        for half in HALVES
    ]
    found = []
    for page in sorted({w[0] for w in every}):
        for half, right_edge in zip(HALVES, edges):
            on_page = [w for w in every if w[0] == page and w[3] > HEADER_BELOW]
            # pdftotext splits a headword with a space or a hyphen in it.
            anchors = [
                line
                for line in into_lines(
                    w for w in on_page if half["term"][0] <= w[1] < half["term"][1]
                )
                if HEADWORD.match(join_line(line))
            ]
            if not anchors:
                continue

            gloss_words = [
                w for w in on_page if half["gloss"][0] <= w[1] < half["gloss"][1]
            ]
            lines = into_lines(gloss_words)

            runs = assign(lines, [centre(a) for a in anchors])
            if runs is None:
                raise SystemExit(
                    f"page {page + 1}: {len(lines)} gloss lines will not divide "
                    f"between {len(anchors)} headwords"
                )
            for anchor, run in zip(anchors, runs):
                found.append((join_line(anchor), join_cell(run, right_edge)))
    return found


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "-"
    found = rows()
    handle = sys.stdout if out == "-" else open(out, "w", encoding="utf-8", newline="")
    writer = csv.writer(handle, lineterminator="\n")
    writer.writerow(["term", "korean"])
    writer.writerows(found)
    if out != "-":
        handle.close()
    missing = sum(1 for _, gloss in found if not gloss)
    print(
        f"{len(found)} headwords, {missing} without a gloss",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
