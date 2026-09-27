# gram

A Vim-flavoured spaced-repetition vocabulary trainer for the terminal, built
around the Word Smart word list.

```
 gram  review  ·  Word Smart                                             Sun 27 Sep
 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  16 left
╭ answer ──────────────────────────────────────────────────────────────────────────╮
│                                                                                  │
│  Abash  (uh BASH)  v                                                             │
│                                                                                  │
│  to make ashamed; to embarrass                                                   │
│                                                                                  │
│    • Meredith felt abashed by her inability to remember her lines in the         │
│      school chorus of "Old McDonald Had a Farm."                                 │
│    • Karl handed in a term paper that he had unabashedly copied from Wikipedia.  │
│                                                                                  │
│    To do something without shame or embarrassment is to do it unabashedly.       │
│                                                                                  │
│    new · ease 2.50 · reps 0 · lapses 0                                           │
╰──────────────────────────────────────────────────────────────────────────────────╯
        y   I know this                    n   I don't know
            next in 1d                         again now · marks it ★
```

## Install

```sh
./install.sh                      # builds, then installs ~/.local/bin/gram
./install.sh --prefix /usr/local  # /usr/local/bin/gram (needs sudo)
./install.sh --uninstall          # remove it again
```

The script tells you what to add to your shell rc if the target directory is
not on your `PATH`. Your collection is never touched, so reinstalling over an
older build is safe.

To build without installing:

```sh
cargo build --release
./target/release/gram
```

## Getting started

The Word Smart decks are already extracted into `decks/`:

```sh
gram import decks/word_smart.csv "Word Smart"                  # 832 cards
gram import decks/word_smart_foreign.csv "Foreign Phrases"     #  33 cards
gram                                                           # start studying
```

Then press `s` to study, `space` to reveal, and `y` / `n` for *I know this* /
*I don't know*. Missed words collect into a study pile you can drill with `M`.
`?` shows every keybinding.

## Keys

Everything is modal and Vim-shaped.

| | |
|---|---|
| `j` `k` `gg` `G` `5j` `Ctrl-d/u` | move |
| `h` `l` `Enter` `q` `Esc` | back / forward |
| `s` | study the selected deck |
| `space` | reveal the answer |
| `y` / `n` | I know this / I don't know |
| `1` `2` `3` `4` | Again / Hard / Good / Easy |
| `m` | mark a word ★ for the study pile |
| `M` | drill the marked words |
| `u` | undo the last answer (or the last delete) |
| `a` `o` `i` `dd` `x` | add / edit / delete / suspend a card |
| `/text` `n` `N` `:noh` | search |
| `:w` `:q` `:wq` `:q!` `ZZ` | write and quit |

Inside the card editor, `i a I A` enter insert mode, `Esc` leaves it, `h l 0 $ w b`
move the cursor, `x D C dd` delete, and `ZZ` saves. `Example` and `Notes` hold
multiple lines; type `\n` to separate them.

## Review modes

By default the answer bar has two buttons:

```
        y   I know this                    n   I don't know
            next in 6d                         again now · marks it ★
```

`y` grades **Good**, `n` grades **Again** — the same SM-2 scheduler underneath,
just a faster decision. `:set grading=sm2` brings back the full Again / Hard /
Good / Easy row; `1`–`4` keep working in either mode when you want the finer
grades.

## The study pile

`m` marks a word ★ as one you still have to learn. Answering **I don't know**
marks it for you, so the pile fills itself as you study; `m` clears the mark
once the word sticks.

```
M                 drill the marked words
:study marked     the same thing
:marked           browse only marked words
:noh              show everything again
```

A marked drill ignores due dates — the point is to pull those words forward and
hammer them. Answers still count, so the schedule keeps moving.

## Commands

```
:deck NAME              create a deck
:rename NAME            rename the current deck
:import FILE [DECK]     import a CSV/TSV
:export FILE            write the current deck out as CSV
:set new=20             new cards introduced per day (per deck)
:set grading=binary|sm2 which answer bar to show (collection-wide)
:forget                 reset this deck's scheduling
:study marked           drill the study pile
:study :browse :marked :stats :help
```

## Scheduling

SM-2, the algorithm behind Anki:

* **Again** drops the ease by 0.20, resets the interval, and puts the card back
  about eight cards later in the same session.
* **Hard** drops the ease by 0.15 and grows the interval by 1.2×.
* **Good** goes 1 day → 6 days → `interval × ease`.
* **Easy** adds 0.15 to the ease and grows the interval by `ease × 1.3`.

Ease never falls below 1.3, and intervals always move forward. Each deck
introduces at most `new_per_day` unseen cards per day, spread evenly through
the day's review backlog rather than front-loaded.

## Data

The collection is one JSON file:

```
$GRAM_DATA  →  $XDG_DATA_HOME/gram/collection.json  →  ~/.local/share/gram/collection.json
```

`gram path` prints the resolved location. Writes go through a temp file and a
rename, so an interrupted save cannot truncate your collection.

## CSV format

Recognised headers, in any order:

```
term, pron, pos, definition, example, notes
```

Without a header row, columns are read as `term, definition, example`. Tabs are
detected from a `.tsv` extension or by sniffing the first line. `example` and
`notes` may hold several lines.

Re-importing matches on the term and **updates** the card rather than
duplicating it, so scheduling survives a corrected export.

## Word Smart extraction

`tools/extract_wordsmart.py` pulls entries out of a Word Smart EPUB:

```sh
python3 tools/extract_wordsmart.py word_smart.epub decks/
```

It reads the A–Z chapters and the foreign-phrases chapter, splitting each
headword paragraph into term, pronunciation, part of speech and definition,
then attaching the bulleted example sentences and the follow-on notes. The
quiz answer key in the back supplies a short gloss (`= step down from power`).
The target word stays wrapped in `*asterisks*` so the TUI can highlight it.

The EPUB itself is not redistributed — point the script at your own copy.

## Development

```sh
cargo test                                   # 64 unit tests
python3 tools/drive_tui.py "s 3" 100 24      # render the TUI headlessly
```

`tools/drive_tui.py` runs gram in a pty, feeds it keystrokes and prints the
resulting screen, which makes the layout testable without a terminal.
