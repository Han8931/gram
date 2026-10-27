# GRE workbook comparison

Compared `sources/GRE단어장.xlsx` column A in all 13 worksheets (including the hidden WordSmart sheet) with headwords in `decks/word_smart.csv` and `decks/GRE_Vocab.csv`. Blank cells and cells without Latin letters were excluded. Matching normalizes Unicode (NFKC), case, and whitespace; it does not merge inflections, spelling variants, or annotations. Definitions and examples in the decks were not treated as headwords. This compares the current CSV decks, not the complete source books.

- Word Smart: 832 distinct headwords.
- GRE_Vocab: 2,336 distinct headwords.
- Combined decks: 2,748 distinct headwords.
- Workbook: 5,649 distinct entries.
- Matched in either deck: 1,796.
- Absent from both decks: 3,853 (68.2%).
- Of the unmatched entries, 3,288 consist only of letters a–z; the rest include phrases, punctuation, accents, or annotations. These counts do not establish how many are distinct vocabulary roots.

| Worksheet | Distinct entries | Absent from both decks |
|---|---:|---:|
| 각종단어장 | 869 | 775 |
| 중국 | 697 | 583 |
| Magoosh | 281 | 229 |
| 거만어 | 1,676 | 861 |
| S.V&W.S | 2,553 | 1,306 |
| KAPLAN | 37 | 34 |
| 최근 | 184 | 150 |
| WordSmart | 1,223 | 485 |
| OfficialTest | 273 | 227 |
| 리딩빈출 | 66 | 61 |
| Manhattan | 216 | 140 |
| 은주씨쎈컴 | 56 | 46 |
| 수학 | 28 | 26 |

Worksheet totals overlap and must not be added.

`gre_xlsx_missing_entries.csv` lists each unmatched entry once, with its original Korean definitions and every source cell location. Entries are preserved without correcting possible typos (for example, `abraision`) or splitting annotations (for example, `color == colour`). Review before importing.
