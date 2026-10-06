#!/usr/bin/env python3
"""Extract vocabulary entries from a TOEFL Power Vocab EPUB into a gram CSV deck.

Usage:  python3 tools/extract_toefl.py toefl_vocab.epub decks/

Output uses gram's import schema:
    term,pron,pos,definition,example,notes

Every headword in the book reads

    ABBREVIATE (uh BREEV ee ate) [~] v to cut short

The bracket is the book's connotation marker — [+] positive, [~] neutral,
[–] negative — which becomes a note on the card, since knowing a word's
colour is half of using it. The target word inside an example sentence stays
wrapped in *asterisks*, the same convention the Word Smart extractor uses, so
the TUI can highlight it.
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

# Paragraph classes that carry entry content. `list0 left` is a quiz column,
# not an example, so examples are matched on the exact class.
HEADWORD_CLASS = "spaceabove"
EXAMPLE_CLASS = "list0"

# A chapter with fewer parsed headwords than this is prose that happens to
# share the word list's paragraph classes — the strategy chapters do.
MIN_ENTRIES = 20

#  ABBREVIATE (uh BREEV ee ate) [~] v to cut short
#  - ANTHROPOLOGY closes its pronunciation with `]`, so accept either bracket
#  - the part of speech is italicised in some entries and bare in others, so
#    emphasis markers are stripped from the headword line before matching
HEAD_RE = re.compile(
    r"""^
        (?P<term>[^(]+?)                       # HEADWORD
        \s*\((?P<pron>[^)\]]*)[)\]]            # (pronunciation)
        \s*(?:\[(?P<tone>[^\]]*)\]\s*)?        # [+] / [~] / [–]
        (?P<pos>[a-z]+)\s+                     # part of speech
        (?P<definition>.+)                     # definition
        $""",
    re.VERBOSE,
)

TONE_WORDS = {
    "+": "positive",
    "~": "neutral",
    "-": "negative",
    "–": "negative",  # en dash, which is what the book sets
    "—": "negative",
}
TONES = {
    symbol: f"{word.capitalize()} connotation."
    for symbol, word in TONE_WORDS.items()
}


def clean(text: str) -> str:
    """Collapse whitespace and normalise the book's typographic quirks."""
    text = html.unescape(text)
    text = text.replace(" ", " ")
    text = re.sub(r"\s+", " ", text)
    return text.strip()


@dataclass
class Block:
    kind: str  # "head" | "example"
    text: str


class BookParser(HTMLParser):
    """Flatten a chapter into ordered head/example blocks.

    <em> spans become *markers* because the book italicises the headword
    wherever it appears in an example sentence.
    """

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.blocks: list[Block] = []
        self._buf: list[str] = []
        self._kind: str | None = None

    # -- helpers ---------------------------------------------------------
    def _open(self, kind: str) -> None:
        self._flush()
        self._kind = kind
        self._buf = []

    def _flush(self) -> None:
        if self._kind is not None:
            text = clean("".join(self._buf))
            text = re.sub(r"\*\s*\*", " ", text)
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
            elif cls == [EXAMPLE_CLASS]:
                self._open("example")
            else:
                self._flush()
        elif tag in ("em", "i"):
            if self._kind:
                self._buf.append("*")
        elif tag == "span":
            # The bullet before an example is an ornament, not text.
            if "list_ornament" in cls:
                self._flush() if self._kind is None else None
        elif tag in ("h1", "h2", "h3", "div", "table", "tr", "td"):
            self._flush()
        elif tag == "br":
            if self._kind:
                self._buf.append(" ")

    def handle_endtag(self, tag: str) -> None:
        if tag == "p":
            self._flush()
        elif tag in ("em", "i"):
            if self._kind:
                self._buf.append("*")

    def handle_data(self, data: str) -> None:
        if self._kind is not None:
            self._buf.append(data)

    def close(self) -> None:  # type: ignore[override]
        super().close()
        self._flush()


@dataclass
class Sense:
    """One headword paragraph. A word the book lists twice — `ESCAPE` as a
    noun and as a verb — is two senses of one card."""

    term: str
    pron: str = ""
    pos: str = ""
    definition: str = ""
    tone: str = ""
    examples: list[str] = field(default_factory=list)


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


def split_headword(text: str) -> Sense | None:
    """Parse one `p.spaceabove` line into term / pron / pos / definition."""
    # Emphasis only ever marks the part of speech here, and one entry opens it
    # mid-word (`a*dj*`), so the markers are no use and in the way.
    m = HEAD_RE.match(text.replace("*", ""))
    if not m:
        return None

    term = clean(m.group("term")).strip(" —-")
    # Headwords are set in caps; prose that shares the class is not.
    if not term or len(term) > 60 or term != term.upper():
        return None

    definition = clean(m.group("definition")).rstrip(";, ")
    if not definition:
        return None

    return Sense(
        term=term.title(),
        pron=clean(m.group("pron") or ""),
        pos=m.group("pos"),
        definition=definition,
        tone=clean(m.group("tone") or ""),
    )


def merge(senses: list[Sense]) -> list[Entry]:
    """One card per word, in the order the book introduces them.

    gram keys cards by term, so the few words listed twice have to become one
    card each — and they are the interesting ones, where a word swaps part of
    speech or connotation between senses.
    """
    grouped: dict[str, list[Sense]] = {}
    for sense in senses:
        grouped.setdefault(sense.term.lower(), []).append(sense)

    entries: list[Entry] = []
    for group in grouped.values():
        first = group[0]
        parts = [s.pos for s in group]
        distinct_pos = list(dict.fromkeys(parts))

        # Label the senses by part of speech only when they differ; repeating
        # "v:" twice tells the reader nothing.
        if len(group) > 1 and len(distinct_pos) == len(group):
            definition = "; ".join(f"{s.pos}: {s.definition}" for s in group)
        else:
            definition = "; ".join(dict.fromkeys(s.definition for s in group))

        entry = Entry(
            term=first.term,
            pron=first.pron,
            pos=", ".join(distinct_pos),
            definition=definition,
        )
        for sense in group:
            entry.examples.extend(sense.examples)

        tones = list(dict.fromkeys(s.tone for s in group))
        if len(tones) == 1:
            note = TONES.get(tones[0])
            if note:
                entry.notes.append(note)
        else:
            # `ESCAPE` is a positive noun and a negative verb; say which.
            spelled = ", ".join(
                f"{s.pos} {TONE_WORDS.get(s.tone, s.tone)}" for s in group
            )
            entry.notes.append(f"Connotation: {spelled}.")
        entries.append(entry)
    return entries


def strip_ornament(text: str) -> str:
    """Drop the bullet the book sets in front of an example."""
    return text.lstrip("•• ").strip()


def parse_chapter(source: str) -> list[Sense]:
    parser = BookParser()
    parser.feed(source)
    parser.close()

    senses: list[Sense] = []
    current: Sense | None = None
    for block in parser.blocks:
        if block.kind == "head":
            current = split_headword(block.text)
            if current:
                senses.append(current)
        elif current is not None:
            example = strip_ornament(block.text)
            if example:
                current.examples.append(example)
    return senses


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


def spine(book: zipfile.ZipFile) -> list[str]:
    """Content documents, in reading order, so A still comes before Z."""
    opf = next((n for n in book.namelist() if n.endswith(".opf")), None)
    if not opf:
        return [n for n in book.namelist() if n.endswith((".xhtml", ".html"))]

    source = book.read(opf).decode("utf-8", "replace")
    base = opf.rpartition("/")[0]
    manifest = dict(
        re.findall(r'<item[^>]*id="([^"]+)"[^>]*href="([^"]+)"', source)
    ) | {
        i: h
        for h, i in re.findall(r'<item[^>]*href="([^"]+)"[^>]*id="([^"]+)"', source)
    }

    names = set(book.namelist())
    order: list[str] = []
    for idref in re.findall(r"<itemref[^>]*idref=\"([^\"]+)\"", source):
        href = manifest.get(idref)
        if not href:
            continue
        for candidate in (f"{base}/{href}" if base else href, href):
            if candidate in names:
                order.append(candidate)
                break
    return order


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    epub, outdir = Path(argv[1]), Path(argv[2])
    if not epub.is_file():
        print(f"no such file: {epub}", file=sys.stderr)
        return 1

    book = zipfile.ZipFile(epub)
    senses: list[Sense] = []
    for name in spine(book):
        source = book.read(name).decode("utf-8", "replace")
        if HEADWORD_CLASS not in source:
            continue
        found = parse_chapter(source)
        # Only the word list holds hundreds of headwords; the chapters that
        # merely quote a few words are not it.
        if len(found) >= MIN_ENTRIES:
            senses.extend(found)

    if not senses:
        print(f"no word list found in {epub}", file=sys.stderr)
        return 1

    entries = merge(senses)
    print("writing decks:")
    write_deck(outdir / "toefl_power_vocab.csv", entries)
    merged = len(senses) - len(entries)
    if merged:
        print(f"  ({merged} second senses merged into the word they belong to)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
