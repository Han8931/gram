#!/usr/bin/env python3
"""Extract vocabulary entries from a Word Smart EPUB into gram CSV decks.

Usage:  python3 tools/extract_wordsmart.py word_smart.epub decks/

Output CSVs use gram's import schema:
    term,pron,pos,definition,example,notes

The target word inside example sentences is kept wrapped in *asterisks*
(the book italicises it), so the TUI can highlight or blank it out.
"""

from __future__ import annotations

import csv
import html
import re
import sys
import zipfile
from dataclasses import dataclass, field
from html.parser import HTMLParser
from pathlib import Path

# Paragraph/list classes that carry entry content.
HEADWORD_CLASS = "extractfont"
EXAMPLE_CLASS = "bullet2"
NOTE_CLASSES = {"nonindent", "indent"}

# `ABASH` (uh BASH) `v` to make ashamed; to embarrass
#  - pronunciation is sometimes inside the <strong>, sometimes after it
#  - a [French—"..."] style gloss may sit between the part of speech and the definition
HEAD_RE = re.compile(
    r"""^\s*
        (?P<term>[^(\[]+?)                    # headword
        (?:\s*\((?P<pron>[^)]*)\))?           # (pronunciation)
        \s*(?:†|\*)?                     # stray daggers
        \s*$""",
    re.VERBOSE,
)
BRACKET_RE = re.compile(r"^\s*\[(?P<origin>[^\]]*)\]\s*")


def clean(text: str) -> str:
    """Collapse whitespace and normalise the book's typographic quirks."""
    text = html.unescape(text)
    text = text.replace(" ", " ")
    text = re.sub(r"\s+", " ", text)
    return text.strip()


@dataclass
class Block:
    kind: str  # "head" | "example" | "note"
    text: str


class BookParser(HTMLParser):
    """Flatten a chapter into ordered head/example/note blocks.

    <em> spans are preserved as *markers* because the book italicises the
    headword wherever it appears in an example sentence.
    """

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.blocks: list[Block] = []
        self._buf: list[str] = []
        self._kind: str | None = None
        self._ul_class: str | None = None
        self._em_depth = 0

    # -- helpers ---------------------------------------------------------
    def _open(self, kind: str) -> None:
        self._flush()
        self._kind = kind
        self._buf = []

    def _flush(self) -> None:
        if self._kind is not None:
            text = clean("".join(self._buf))
            # collapse adjacent/empty emphasis produced by nested tags
            text = re.sub(r"\*\s*\*", " ", text)
            text = re.sub(r"\*(\s*)\*", r"\1", text)
            if text:
                self.blocks.append(Block(self._kind, text))
        self._kind = None
        self._buf = []

    # -- HTMLParser hooks ------------------------------------------------
    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        a = dict(attrs)
        cls = (a.get("class") or "").split()
        if tag == "p":
            if HEADWORD_CLASS in cls:
                self._open("head")
            elif NOTE_CLASSES & set(cls):
                self._open("note")
            else:
                self._flush()
        elif tag == "ul":
            self._ul_class = " ".join(cls)
            self._flush()
        elif tag == "li":
            if self._ul_class and EXAMPLE_CLASS in self._ul_class.split():
                self._open("example")
            else:
                self._flush()
        elif tag in ("em", "i"):
            if self._kind:
                self._em_depth += 1
                self._buf.append("*")
        elif tag in ("h1", "h2", "h3", "div", "table", "tr", "td"):
            self._flush()
        elif tag == "br":
            if self._kind:
                self._buf.append(" ")

    def handle_endtag(self, tag: str) -> None:
        if tag in ("p", "li"):
            self._flush()
        elif tag == "ul":
            self._ul_class = None
            self._flush()
        elif tag in ("em", "i"):
            if self._kind and self._em_depth > 0:
                self._em_depth -= 1
                self._buf.append("*")

    def handle_data(self, data: str) -> None:
        if self._kind is not None:
            self._buf.append(data)

    def close(self) -> None:  # type: ignore[override]
        super().close()
        self._flush()


@dataclass
class Entry:
    term: str
    pron: str = ""
    pos: str = ""
    definition: str = ""
    examples: list[str] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)

    def as_row(self) -> dict[str, str]:
        return {
            "term": self.term,
            "pron": self.pron,
            "pos": self.pos,
            "definition": self.definition,
            "example": "\n".join(self.examples),
            "notes": "\n".join(self.notes),
        }


# A handful of headword paragraphs run the definition straight into an
# explanatory sentence:  "...; bottomless An *abyss* (uh BIS) is a pit..."
# The seam is a capitalised word starting what is clearly a full sentence.
SEAM_RE = re.compile(r"(?<=[a-z;,)*])\s+(?=[A-Z])")


def split_trailing_note(definition: str) -> tuple[str, str]:
    """Peel an explanatory sentence off the end of a definition."""
    for m in SEAM_RE.finditer(definition):
        head, tail = definition[: m.start()], definition[m.end():]
        # Real definitions are fragments; an explanation is a sentence that
        # ends in a full stop. Proper nouns ("Republic of South Africa")
        # fail both the length and the terminator test.
        if len(tail) >= 40 and tail.endswith((".", "!", "?")):
            return head.rstrip(";, "), tail
    return definition, ""


def split_headword(text: str) -> Entry | None:
    """Parse one `p.extractfont` line into term / pron / pos / definition."""
    # The part of speech is the first *emphasised* token after the headword.
    parts = text.split("*")
    if len(parts) < 3:
        return None

    head = parts[0]
    pos = parts[1].strip()
    rest = "*".join(parts[2:])

    m = HEAD_RE.match(head)
    if not m:
        return None
    term = clean(m.group("term")).strip(" —-")
    pron = clean(m.group("pron") or "")

    if not term or len(term) > 60:
        return None
    # Headwords are set in caps in the book; anything else is prose.
    if not re.search(r"[A-ZÀ-Ü]", term):
        return None

    origin = ""
    bm = BRACKET_RE.match(rest)
    if bm:
        origin = clean(bm.group("origin"))
        rest = rest[bm.end():]

    definition = clean(rest).lstrip("—- ").strip()
    definition = definition.rstrip(";, ")
    if not definition:
        return None

    definition, trailing = split_trailing_note(definition)

    entry = Entry(term=term.title() if term.isupper() else term,
                  pron=pron, pos=pos, definition=definition)
    if origin:
        entry.notes.append(f"[{origin}]")
    if trailing:
        entry.notes.append(trailing)
    return entry


def parse_chapter(source: str) -> list[Entry]:
    parser = BookParser()
    parser.feed(source)
    parser.close()

    entries: list[Entry] = []
    current: Entry | None = None
    for block in parser.blocks:
        if block.kind == "head":
            parsed = split_headword(block.text)
            if parsed:
                entries.append(parsed)
                current = parsed
            else:
                current = None
        elif current is None:
            continue
        elif block.kind == "example":
            current.examples.append(block.text)
        elif block.kind == "note":
            # Notes introduce related forms ("To do something without shame...").
            if len(block.text) > 400:
                continue
            current.notes.append(block.text)
    return entries


def short_glosses(source: str) -> dict[str, str]:
    """Quiz answers give a one-word gloss: `abash—embarrass`."""
    glosses: dict[str, str] = {}
    for raw in re.findall(r"<td[^>]*>(.*?)</td>", source, re.S):
        text = clean(re.sub(r"<[^>]+>", "", raw))
        m = re.match(r"^(?:[a-z]\s+)?([A-Za-zà-ü'\- ]+?)\s*—\s*(.+)$", text)
        if m:
            glosses.setdefault(m.group(1).strip().lower(), m.group(2).strip())
    return glosses


def write_deck(path: Path, entries: list[Entry]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(
            fh, fieldnames=["term", "pron", "pos", "definition", "example", "notes"]
        )
        writer.writeheader()
        for entry in entries:
            writer.writerow(entry.as_row())
    print(f"  {path}  ({len(entries)} cards)")


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    epub, outdir = Path(argv[1]), Path(argv[2])
    if not epub.is_file():
        print(f"no such file: {epub}", file=sys.stderr)
        return 1

    book = zipfile.ZipFile(epub)
    names = book.namelist()

    def read(fragment: str) -> str | None:
        for name in names:
            if fragment in name:
                return book.read(name).decode("utf-8", "replace")
        return None

    # Chapter 13 holds the quiz answer key, which doubles as a short gloss.
    answers = read("_c13_")
    glosses = short_glosses(answers) if answers else {}

    az: list[Entry] = []
    for letter in "abcdefghijklmnopqrstuvwxyz":
        source = read(f"_c02{letter}_")
        if source:
            az.extend(parse_chapter(source))

    for entry in az:
        gloss = glosses.get(entry.term.lower())
        if gloss and gloss.lower() not in entry.definition.lower():
            entry.notes.insert(0, f"= {gloss}")

    print("writing decks:")
    write_deck(outdir / "word_smart.csv", az)

    foreign = read("_c11_")
    if foreign:
        write_deck(outdir / "word_smart_foreign.csv", parse_chapter(foreign))

    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
