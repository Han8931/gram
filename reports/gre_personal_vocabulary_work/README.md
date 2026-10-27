# GRE Personal Vocabulary — completed authoring records

All 3,853 source entries are drafted. The final deck contains 3,851 cards after two merges. See [the completed report](../gre_personal_vocabulary.md) and [the final deck](../../decks/GRE_Personal_Vocabulary.csv).

`draft_a.jsonl` through `draft_e.jsonl` are the final authoring records used by `build.py`. IDs are zero-based row positions in `../gre_xlsx_missing_entries.csv`. JSONL records contain id, term, pos, definition, example, and usage_note. Original Korean notes and source locations are joined by the builder.

The PSV files and `../GRE_Personal_Vocabulary_DRAFT.csv` are earlier drafting history, not the final deck. `remaining.csv` has no data rows because no source entries remain undrafted.

Rebuild with:

```sh
python3 reports/gre_personal_vocabulary_work/build.py
```

This regenerates the final CSV, audit, and summary without importing or changing study data. For any future content correction, update the corresponding JSONL record and rebuild. English definitions and examples are newly written; reference links in usage notes document selected lexical checks.
