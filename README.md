# gram

[![ci](https://github.com/Han8931/gram/actions/workflows/ci.yml/badge.svg)](https://github.com/Han8931/gram/actions/workflows/ci.yml)

A Vim-flavoured spaced-repetition vocabulary trainer for the terminal, built
around the Word Smart word list.

```
 gram  review  ·  Word Smart                        review needed (16)  ·  Mon 05 Oct
   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━   20%  4 of 20 done  · 1 retrying
  ╭ answer ────────────────────────────────────────────────────────────────────────╮
  │                                                                                │
  │  Absolve  (ab ZOLV)  v                                                         │
  │                                                                                │
  │  to forgive or free from blame; to free from sin; to free from an obligation    │
  │                                                                                │
  │    • The priest absolved the sinner who had come to church to confess.          │
  │                                                                                │
  │    • Tom's admission of guilt absolved Mary, who had originally been accused    │
  │      of the crime.                                                              │
  │                                                                                │
  │    • Jake absolved Ciara of her obligation to go to the prom with him; he       │
  │      told her it was all right if she went with the captain of the football     │
  │      team instead.                                                              │
  │                                                                                │
  │    │ It is also possible to absolve someone of a responsibility.                │
  ╰─────────────────────────────────────────────────────────────────────────── ↓ j ╯
              y   I know this                            n   I don't know
                next in 1d                           again now · marks it ★
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

`decks/` holds the extracted word lists and `data/collection.json` the
collection built from them, so a clone comes with the decks and their
schedules — and, because that word list is copyrighted, this repository stays
private. From a clean start, import what you want to study:

```sh
gram import decks/word_smart.csv "Word Smart"                  # 832 cards
gram import decks/word_smart_foreign.csv "Foreign Phrases"     #  33 cards
gram import decks/chinese_basic.csv "Chinese Basics"           # 152 cards
gram import decks/japanese_basic.csv "Japanese Basics"         # 154 cards
gram                                                           # start studying
```

Rebuilding a deck from the book needs your own copy of the EPUB:

```sh
python3 tools/extract_wordsmart.py word_smart.epub decks/
```

Any CSV works, though — see [CSV format](#csv-format) if you would rather bring
your own list.

Then press `s` to study, `space` to reveal, and `y` / `n` for *I know this* /
*I don't know*. Missed words pick up a `★ marked` status you can drill with
`M`. `?` shows every keybinding.

## Deck status

Every deck carries a status, in the deck list and in the top-right of every
screen, so you can tell what wants doing without reading the numbers:

| | |
|---|---|
| `review needed` | cards are due, or waiting to be relearned |
| `new cards` | nothing due, but today's ration of unseen cards is untouched |
| `up to date` | everything for today is done |
| `empty` | no cards in the deck |

`gram decks` prints the same column from the shell.

## Keys

Everything is modal and Vim-shaped.

| | |
|---|---|
| `j` `k` `gg` `G` `5j` `5G` `Ctrl-d/u` `{` `}` | move — or scroll a long answer while reviewing |
| `h` `l` `Enter` `q` `Esc` | back / forward |
| `s` | study the selected deck |
| `space` | reveal the answer |
| `y` / `n` | I know this / I don't know |
| `1` `2` `3` `4` | Again / Hard / Good / Easy |
| `m` `Ctrl-m` | cycle this word's status |
| `z` | hide / show every meaning in the browser |
| `M` | drill the words carrying a status |
| `u` `Ctrl-r` | undo / redo a change — or undo the last answer, while reviewing |
| `a` `o` `i` `dd` `x` | add / edit / delete / suspend a card |
| `/text` `n` `N` | jump to a word as you type it, then step through the matches |
| `f` `:noh` | filter the list as you type, then show everything again |
| `:w` `:q` `:wq` `:qa` `ZZ` | write and quit |
| `:q!` `:qa!` `ZQ` | quit without saving |

`q` and `Esc` go back one screen, and the deck list is the last one: pressing
either there writes your changes and exits, the same as `ZZ`. Worth knowing
before you lean on `u`, which only lasts as long as the program does — a
delete you walk out of the deck list on is a delete you have saved. `:q!`
leaves the collection alone, asking first if there is anything to lose.

Inside the card editor, `i a I A` enter insert mode, `Esc` leaves it, `h l 0 $ w b`
move the cursor, `x D C dd` delete, and `ZZ` saves. `Example` and `Notes` hold
multiple lines; type `\n` to separate them, and the editor shows them as the
line breaks they are.

Each field is laid out over as many lines as its text needs, so a long example
is readable rather than running off the edge: `Term`, `Pron` and `Pos` keep one
line, `Definition` up to three, and `Example` and `Notes` up to eight each. The
dialog is only as tall as the card it holds. Where a field still holds more
than fits, `…` says so, and the field scrolls to follow the cursor as you edit.

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

The bar above the card counts cards seen off, not answers given. A word you
answer **I don't know** comes back later in the session and shows up as
`retrying` until you get it right, so the bar never reads 100% with cards still
in the queue. Long answers scroll with `j` / `k`, and the card's bottom border
shows `↓ j` while there is more below.

## Word status

Every word carries a status of your own, in the browser's `STATUS` column and
beside the term while reviewing. `m` steps to the next one:

```
—  ->  ◆ review  ->  ★ marked  ->  ▲ test  ->  —
```

`None` is in the cycle, so a status can always be taken off again. `Ctrl-m`
steps backwards, and `:status review|marked|test|none` sets one outright.

| | |
|---|---|
| `—` | no status |
| `◆ review` | worth another look |
| `★ marked` | in the study pile |
| `▲ test` | ready to test yourself on |

This is **yours to set**, unlike the `STATE` column (`new` / `learn` / `rev`),
which only the scheduler moves — see [Scheduling](#scheduling). Answering
**I don't know** sets `★ marked` for you, but only on a word you have not
labelled yourself, so it never overwrites a deliberate choice.

```
M                 drill every word carrying a status
:study marked     drill just one status (review / marked / test)
:marked           browse everything flagged
:marked test      browse just one status
:noh              show everything again
```

A status drill ignores due dates — the point is to pull those words forward and
hammer them. Answers still count, so the schedule keeps moving.

Collections written before statuses existed are migrated on load: a word that
was `marked` becomes `★ marked`.

## Browsing

`Enter` on a deck opens the browser: one row per word, with the `STATUS`,
`STATE` and `DUE` columns beside it. The selected word is also shown in full —
definition, examples and notes, laid out as they are in review — in a `card`
pane below the list, so the pane gets the whole width for prose and the list
keeps its columns. Terminals under 22 rows skip it and give everything to the
list. A `…` in the pane's border means the word has more text than fits.

`Enter` opens the word on its own, laid out the way a revealed review card is
— definition, then its examples, notes and look-alikes under headings — at a
width prose is comfortable to read at. `j` / `k` scroll a long one, `J` / `K`
(or `Tab` / `Shift-Tab`) step to the next or previous word without going back
to the list, `m` and `x` work as they do in the list, `i` opens the editor,
and `q` goes back with the cursor on whichever word you ended on.

### Finding a word

`/` moves the cursor; `f` narrows the list. Both answer as you type, before
`Enter`, and `Esc` puts things back the way they were.

```
/abd       the cursor jumps to Abdicate; the list stays whole
n  N       the next / previous match, best first
f abd      only the matching words are listed, best first
:noh       forget the search and the filter
```

Neither needs the spelling right. A query is matched case- and
accent-insensitively — `naive` finds `naïve`, and `ni hao` or `nihao` finds a
card read `nǐ hǎo` — against the term, its reading, and then the prose. Matches
are ranked: the word itself, then words starting with what you typed, then the
text anywhere in a term, then a definition or example that mentions it. When
no term matches as typed, words within a typo or two are offered instead —
one for a query of four to six letters, two up to ten, three beyond — so
`ubiqitus` still lands on Ubiquitous, and the status line says the match was a
close spelling rather than an exact one. Queries under four letters are taken
literally.

### Practising from the list

A browser that shows every definition is an answer sheet, which is no use for
testing yourself against your own deck. `z` covers them all:

```
   # TERM                   POS  DEFINITION          STATUS    STATE  DUE
▌  1 Abash                  v    ············        —         rev    due
   2 Abate                  v    ············        —         rev    due
   3 Abdicate               v    ············        —         rev     6d
```

The words keep their pronunciation and part of speech; the `DEFINITION` column
and the `card` pane are both covered, examples included — a sentence using the
word gives away as much as the gloss does. `z` again brings everything back.
The border reads `hidden` while it is on, and nothing is written to your
collection: it is a way of looking at the list, not a setting, so it is gone
the next time you open the deck.

Work down the list recalling each meaning, and press `space` to uncover just
the word under the cursor when you want to check yourself. Moving off it covers
it again, so the row below is never already answered.

Nothing here grades anything or touches a due date. For answers that count
towards the schedule, study the deck with `s`, or drill a status with `M`.

## Commands

```
:deck NAME              create a deck
:rename NAME            rename the current deck
:import FILE [DECK]     import a CSV/TSV
:export FILE            write the current deck out as CSV
:set new=20             new cards introduced per day (per deck)
:set grading=binary|sm2 which answer bar to show (collection-wide)
:forget                 reset this deck's scheduling
:status NAME            set this word's status (review|marked|test|none)
:study NAME             drill one status
:study more [N]         study N extra new cards today (default: a day's worth)
:marked [NAME]          browse flagged words, or just one status
:restore [N|FILE]       list kept collection backups, or put one back
:study :browse :marked :stats :help
```

`Tab` completes what you are typing: command names first, then their
arguments — statuses for `:status` and `:marked`, scopes for `:study`, keys and
values for `:set`, and file paths for `:import`, `:export` and `:restore`. The
first press fills in as far as the candidates agree, so `:st`+`Tab` offers
`stats`, `status` and `study` in a strip above the command line; after that
each `Tab` steps to the next and `Shift-Tab` back. A completion that ends in
`/` or `=` — a directory, or `grading=` — is left without a trailing space so
the next `Tab` carries straight on into it.

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

To push further on a given day, `:study more` runs the due cards plus another
day's worth of new ones (at least 10), and `:study more 30` asks for exactly
30 extra. It is a one-off: `new_per_day` stays as it was, and tomorrow's ration
is the usual one. Use `:set new=N` to change the limit for good.

### Changing `due` and `state`

No key or command edits one card's `due` or `state` on its own. They move three
ways:

* **Answering the card.** **Again** puts it in `learn`, due today; the other
  three grades put it in `rev` and push `due` out by the new interval.
* **`:forget`** (alias `:reset`) on the current deck. Every card goes back to
  `new`, due today, ease 2.50, with interval, reps and lapses cleared and
  today's new-card ration reopened. It asks before doing it.
* **By hand in the collection JSON.** Quit gram first — it rewrites the whole
  file on save and would overwrite the edit.

```json
{ "term": "laconic", "state": "review", "due": "2026-10-12",
  "interval": 7, "ease": 2.5, "reps": 3, "lapses": 0 }
```

`state` is `new`, `learning` or `review`; `due` is `YYYY-MM-DD`. Keep the two
agreeing with each other: a `new` card is never due whatever its date, and
`interval` should match the gap you set, or the next answer will jump from the
old number.

To take a word out of the rotation without disturbing its schedule, suspend it
with `x` instead — a suspended card is skipped everywhere and comes back with
its interval intact.

## Undo

`u` takes back the last change you made to the collection, Vim-style, and
`Ctrl-r` puts it back:

```
u          undo the last change            3u          undo three changes
Ctrl-r     redo it                         3Ctrl-r     redo three
```

Not just deletes. Edits, statuses, suspends, `:forget`, a deleted deck, a
rename, an import, `:set` — every change goes on one history, 64 deep. Each
step says what it was (`undid deleting Abash`) and takes you back to the screen
and the row where it happened, so undoing a delete you made ten words ago puts
you in front of the word again. Making a change after an undo closes off the
redo, as in Vim.

While reviewing, `u` keeps the finer meaning it always had — take back my last
answer, with the card's schedule and its place in the queue — because the
session queue is not part of the collection and a snapshot could not restore
it. Everything else is on the change history.

The history lives in the running program and is not written to your collection,
so quitting ends it: **a change you have saved and quit on is not undoable by
`u`.** What covers you there is a conflict copy and `:restore`, below — and
those are only written when gram spots a competing version on disk, not on
every save. If you have deleted something, saved, and quit, your own backups
of `collection.json` are the only way back.

## Data

The collection is one JSON file:

```
$GRAM_DATA  →  $XDG_DATA_HOME/gram/collection.json  →  ~/.local/share/gram/collection.json
```

`gram path` prints the resolved location. In this checkout
`$XDG_DATA_HOME/gram` is a symlink to `data/`, so the collection gram writes is
the tracked `data/collection.json` and studying shows up as a diff. Note the
symlink is on the *directory*: a save renames a temp file into place, which
would replace a symlinked collection file with a regular one on the first
write.

Writes go through a temp file and a
rename, so an interrupted save cannot truncate your collection. The temp file
is flushed to disk before the rename and the directory after it, so a power
cut cannot leave the new name pointing at a file whose contents never made it
out of the cache. The temp file
carries the writing process's pid, so two gram instances cannot rename each
other's half-written file into place.

If the collection on disk has changed since gram loaded it — `gram import` run
in another terminal, or a second instance — the save keeps that version beside
the collection as `collection.conflict-<date>-<time>.json` and says so, rather
than overwriting it silently. The copy in the running program still wins: it is
the one you have been studying. Repeated conflicts inside the same second get
a counter (`…-203010-1.json`) so one backup never lands on another. If the
copy cannot be made at all — a full or read-only directory — the save is
refused and the error says so; nothing is overwritten that was not first kept.

`:restore` lists those copies by stamp, newest first — the shared
`collection.conflict-` … `.json` around them is left off, since it is the
same for every one:

```
:restore                1: 20261006-203010-1  2: 20261006-203010
:restore 1              put the newest one back
:restore FILE           put any readable collection back
```

A restore asks before it acts, reads the backup before trusting it, and keeps
whatever it displaces the same way — so restoring the wrong copy is itself
undoable. It ends any session in progress: card ids from the old collection
mean nothing against a different set of cards.

Studying checkpoints itself every ten answers, on top of the save at the end of
a session, so a crash or a closed terminal costs a few answers rather than the
whole sitting. `:w` still writes on demand, and `ZZ` writes on the way out.

Answers are appended to a review log, which the statistics screen reads. It is
capped at the most recent 50,000 answers so the file cannot grow without
bound; the rates below are measured over whatever the log still holds.

## Statistics

`S` opens the statistics screen. Two of the numbers there are easy to confuse:

| | |
|---|---|
| `accuracy` | share of **every** answer that was not Again — drills, same-session retries and first sightings of new words included |
| `retention` | share of **scheduled reviews answered right the first time** — one answer per card per day, with drills and brand-new words left out |

Accuracy says how the sessions went. Retention says whether the schedule is
working: if it sits low, the intervals are growing faster than your memory.
Collections logged before gram recorded which answers came from drills count
as scheduled reviews, which is what they were unless you were drilling.

## Colour

gram uses fixed entries from the 256-colour palette rather than the terminal's
sixteen named colours, so secondary text stays readable whatever scheme the
terminal is using. Set [`NO_COLOR`](https://no-color.org) to turn colour off:
selected rows and badges are then shown in reverse video.

## Non-Latin text

Cards are measured in terminal columns, not characters, so Korean, Japanese and
Chinese text lines up: `한국어` is six columns wide, not three, and a letter
carrying a combining accent is one, not two. Tables pad and truncate by column,
prose wraps by column, and the cursor sits where the text actually is.

Editing counts the same way. `é` typed as `e` plus a combining acute is one
character to every motion and one backspace to delete, and each Korean syllable
is a single step for `h`, `l`, `w` and `b`.

`tools/drive_tui.py` models double-width cells too, so the layout tests mean
something for non-Latin decks.

## CSV format

Recognised headers, in any order:

```
term, pron, pos, definition, example, notes, confusing
```

Without a header row, columns are read as `term, definition, example`. Tabs are
detected from a `.tsv` extension or by sniffing the first line. `example`,
`notes` and `confusing` may hold several lines.

`confusing` lists words easily mistaken for this one, one per line, each
optionally followed by how it differs (`effect — a result, usually a noun`).
They close the card under a *don't confuse with* heading. The column may also
be headed `confusable`, `confused with`, `lookalike` or `similar`, and in the
card editor it is the last field.

Re-importing matches on the term and **updates** the card rather than
duplicating it, so scheduling survives a corrected export.

An import is all-or-nothing: the file is read onto a staged copy of the deck
and committed only once it has parsed, so a file that turns out to be
unreadable or empty leaves your collection — and the deck list — untouched.

## Building decks from a book

Two extractors live in `tools/`, one per book. Both write gram's CSV schema and
leave the target word wrapped in `*asterisks*` inside example sentences, so the
TUI can highlight it.

```sh
python3 tools/extract_wordsmart.py word_smart.epub decks/   # 832 + 33 cards
python3 tools/extract_toefl.py toefl_vocab.epub decks/      # 799 cards
```

A CSV under `decks/` is not yet a deck: nothing is bundled, and the deck list
shows your collection, not the directory. Import what you want to study.

```sh
gram import decks/toefl_power_vocab.csv "TOEFL Power Vocab"
```

**Chinese and Japanese basics.** Two hand-written starter decks of about 150
everyday words each: the whole HSK 1 list for Chinese, and core JLPT N5 words
for Japanese — greetings, pronouns, verbs, adjectives, time words, numbers and
counters, and the common particles. `pron` holds pinyin with tone marks for Chinese, and
the kana reading plus romaji for Japanese. Each example sentence carries its
English translation, and the Chinese notes give the traditional form where it
differs from the simplified one.

**Word Smart.** Reads the A–Z chapters and the foreign-phrases chapter,
splitting each headword paragraph into term, pronunciation, part of speech and
definition, then attaching the bulleted example sentences and the follow-on
notes. The quiz answer key in the back supplies a short gloss
(`= step down from power`).

**TOEFL Power Vocab.** Reads the word-list chapter — found by looking for the
document with hundreds of parsable headwords, rather than by filename — and
turns each entry into a card:

```
ABBREVIATE (uh BREEV ee ate) [~] v to cut short
```

The bracket is the book's connotation marker, and it becomes a note on the
card: `Positive connotation.`, `Neutral connotation.`, `Negative connotation.`
Knowing a word's colour is half of using it, and the book marks every one.

Five words are listed twice, once per sense. gram keys cards by term, so those
become one card carrying both senses, labelled by part of speech, with both
example sentences and both connotations where they differ:

```
Escape  (i SKEYP)  n, v
n: a distraction or relief from routine; v: to get away from; to avoid a
threatening evil
  • Camping in the outdoors is a great escape from modern life.
  • The prisoner escaped from the jail through a hole he had dug in his cell.
  │ Connotation: n positive, v negative.
```

Neither EPUB is redistributed — point the scripts at your own copies, which
stay git-ignored. The extracted `decks/` are tracked, as is the collection
built from them, so **keep this repository private.**

## Development

```sh
cargo test                                   # 138 unit tests
python3 tools/check_layout.py                # render at three sizes, check widths
python3 tools/drive_tui.py "s 3" 100 24      # render the TUI headlessly
```

CI runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, a CLI
smoke test and `tools/check_layout.py` on every push and pull request.

`tools/drive_tui.py` runs gram in a pty, feeds it keystrokes and prints the
resulting screen, which makes the layout testable without a terminal. It uses
`target/release/gram` if that exists and `target/debug/gram` otherwise;
`$GRAM_BIN` overrides. Point `$GRAM_DATA` at a scratch file so a test run
never touches your real collection.

`tools/check_layout.py` drives it: it builds its own scratch collection with a
Korean deck, renders six screens at 60x20, 80x24 and 100x30, and fails if any
row overflows the screen width or an expected string is missing. 한 is two
columns wide, so code that counts characters where it should count columns
pushes a border past the edge — which no unit test notices.
