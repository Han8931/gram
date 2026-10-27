# GRE Personal Vocabulary

A personal study deck built from the entries in `sources/GRE단어장.xlsx` that did not match headwords in the existing Word Smart and GRE Vocab CSV decks.

- 3,853 source entries accounted for.
- 3,851 cards, each with an English definition and an original example sentence.
- 2 duplicate entries merged after normalization: `according` into `according to`, and the annotated contrast pattern into `however`.
- 86 source entries renamed or cleaned up, including spelling corrections, spacing, grammar placeholders, and incomplete phrase annotations.
- Original Korean notes and worksheet cell references retained on every card.
- Part-of-speech labels standardized; example target words are highlighted with asterisks for gram.
- Pronunciation fields are blank. No pronunciation guesses were added.

## Files and use

The finished deck is [GRE_Personal_Vocabulary.csv](../decks/GRE_Personal_Vocabulary.csv). Import it under the name **GRE Personal Vocabulary**:

```sh
gram import decks/GRE_Personal_Vocabulary.csv "GRE Personal Vocabulary"
```

The [entry audit](gre_personal_vocabulary_audit.csv) maps every original entry to its final headword and records corrections and merged entries. The earlier `GRE_Personal_Vocabulary_DRAFT.csv` is an archived, incomplete first draft; use the finished deck above.

## Editorial choices

Useful phrases, inflections, grammar patterns, and mathematics vocabulary were retained because this is a record of personal learning, not a standardized GRE frequency list. Definitions follow the intended senses in the Korean notes where possible; misleading glosses are corrected and explained in usage notes. Original Korean wording is preserved as historical context, not silently rewritten.

Seven corrected headwords also occur in the existing decks: **flippant, grievous, heart-rending, propagate, remorseless, standoffish, undeterred**. Their personal source notes remain in this deck, and the overlap is identified in each card and the audit. Existing decks were not altered to merge these notes.

Definitions and examples were newly drafted. Selected uncommon words and uncertain usages were checked against dictionaries; the relevant cards contain reference links. These were targeted checks, not a dictionary verification of every card.

## Validation

Coverage checks account for every source ID exactly once before duplicate merging. The completed CSV has unique normalized headwords, nonempty English definitions and examples, and balanced example highlighting. A trial import into a copy of the collection added all 3,851 cards without changing the existing decks, review schedules, review log, or grading settings. The live import also added all 3,851 cards successfully; a post-import comparison confirmed that every pre-existing deck, card, schedule, review-log entry, and grading setting remained unchanged.

The authoring records and reproducible builder are retained in `gre_personal_vocabulary_work/`. Running `python3 reports/gre_personal_vocabulary_work/build.py` regenerates the CSV and audit without changing the study collection.
