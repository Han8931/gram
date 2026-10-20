//! Finding a word you cannot quite spell.
//!
//! A query is folded — lowercased, accents and tone marks dropped — and then
//! tried against each card in tiers, best first: the term itself, the start of
//! the term or of its reading, anywhere in the term, anywhere in the prose,
//! and last a misspelling of the term. Each card gets the best tier it
//! reaches, so `abash` lists Abash before the sentence that mentions it, and
//! `abesh` still finds Abash at all.
//!
//! Misspellings are a fallback, not a bonus: once some term matches what was
//! typed, words that are merely one letter off it stay out of the way.

use crate::model::Card;

/// How well a card matched; lower is better. Compares tier first, then how
/// many typos the fuzzy tier had to forgive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Score {
    tier: u8,
    typos: u8,
}

impl Score {
    /// Matched by spelling the word wrong, rather than by finding the text.
    pub fn is_fuzzy(self) -> bool {
        self.tier == FUZZY
    }

    /// The text was found in the word itself, not in its prose.
    fn in_term(self) -> bool {
        self.tier < PROSE
    }
}

/// Order what matched, best first, ties left in the order given — and drop
/// the near misses when there are real hits on a term to show instead.
pub fn rank<T: Ord>(mut scored: Vec<(Score, T)>) -> Vec<T> {
    scored.sort();
    if scored.iter().any(|(score, _)| score.in_term()) {
        scored.retain(|(score, _)| !score.is_fuzzy());
    }
    scored.into_iter().map(|(_, item)| item).collect()
}

const EXACT: u8 = 0;
const PREFIX: u8 = 1;
const WORD_PREFIX: u8 = 2;
const INSIDE: u8 = 3;
const PROSE: u8 = 4;
const FUZZY: u8 = 5;

/// What was typed, folded once rather than once per card.
pub struct Query {
    text: String,
    /// The same with the spaces taken out, so `nihao` finds `nǐ hǎo`.
    compact: String,
}

impl Query {
    pub fn new(raw: &str) -> Self {
        let text = fold(raw.trim());
        let compact = text.split_whitespace().collect();
        Self { text, compact }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// How `card` matches, or `None` when it does not.
    pub fn score(&self, card: &Card) -> Option<Score> {
        if self.is_empty() {
            return None;
        }
        let term = fold(&card.term);
        let pron = fold(&card.pron);
        let pron_compact: String = pron.split_whitespace().collect();
        let at = |tier| Some(Score { tier, typos: 0 });

        if term == self.text {
            return at(EXACT);
        }
        if term.starts_with(&self.text) {
            return at(PREFIX);
        }
        let word_start = |field: &str| {
            field
                .split(|c: char| c.is_whitespace() || c == '-' || c == '/')
                .any(|word| word.starts_with(&self.text))
        };
        if word_start(&term)
            || word_start(&pron)
            || (!self.compact.is_empty() && pron_compact.starts_with(&self.compact))
        {
            return at(WORD_PREFIX);
        }
        if term.contains(&self.text) || pron.contains(&self.text) {
            return at(INSIDE);
        }
        let prose = [
            &card.definition,
            &card.example,
            &card.notes,
            &card.confusing,
        ];
        if prose.iter().any(|field| fold(field).contains(&self.text)) {
            return at(PROSE);
        }

        let allowed = typo_budget(self.compact.chars().count());
        if allowed == 0 {
            return None;
        }
        // Against the whole term, and each word of it, so a typo in the second
        // word of a phrase still lands. The reading counts too: a Chinese
        // card is found by its pinyin.
        let mut candidates: Vec<&str> = vec![&term];
        candidates.extend(term.split_whitespace());
        if !pron_compact.is_empty() {
            candidates.push(&pron_compact);
        }
        let query: Vec<char> = self.compact.chars().collect();
        let typos = candidates
            .iter()
            .map(|candidate| prefix_distance(&query, candidate))
            .min()?;
        (typos <= allowed).then_some(Score {
            tier: FUZZY,
            typos: typos as u8,
        })
    }
}

/// Typos forgiven for a query this many letters long. Under four letters
/// there is too little to go on — `cat` is one typo from half the deck.
fn typo_budget(letters: usize) -> usize {
    match letters {
        0..=3 => 0,
        4..=6 => 1,
        7..=10 => 2,
        _ => 3,
    }
}

/// Fewest edits that turn `query` into the start of `text`: what is still to
/// be typed is free, so a half-finished, misspelt word scores on what it has.
/// Optimal string alignment, so a swapped pair (`abahs`) is one edit, not two.
fn prefix_distance(query: &[char], text: &str) -> usize {
    let text: Vec<char> = text.chars().collect();
    let (rows, cols) = (query.len(), text.len());
    // Three rows of the table are all the transposition rule ever looks at.
    let mut before: Vec<usize> = vec![0; cols + 1];
    let mut previous: Vec<usize> = (0..=cols).collect();
    let mut current: Vec<usize> = vec![0; cols + 1];
    for i in 1..=rows {
        current[0] = i;
        for j in 1..=cols {
            let cost = usize::from(query[i - 1] != text[j - 1]);
            let mut best = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + cost);
            if i > 1 && j > 1 && query[i - 1] == text[j - 2] && query[i - 2] == text[j - 1] {
                best = best.min(before[j - 2] + 1);
            }
            current[j] = best;
        }
        std::mem::swap(&mut before, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }
    // `previous` now holds the last row: the query against every prefix.
    previous.into_iter().min().unwrap_or(rows)
}

/// Lowercase, with accents and tone marks taken off, so `naive` finds
/// `naïve` and `ni hao` finds `nǐ hǎo`. Letters with no plain form, and every
/// non-Latin script, pass through as they are.
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter_map(|ch| {
            // A combining accent typed on its own goes with nothing.
            if ('\u{300}'..='\u{36f}').contains(&ch) {
                return None;
            }
            Some(plain(ch))
        })
        .collect()
}

fn plain(ch: char) -> char {
    const TABLE: &[(&str, char)] = &[
        ("àáâãäåāăąǎǟǡǻȁȃȧ", 'a'),
        ("çćĉċč", 'c'),
        ("ďđ", 'd'),
        ("èéêëēĕėęěȅȇȩ", 'e'),
        ("ĝğġģǧ", 'g'),
        ("ĥħȟ", 'h'),
        ("ìíîïĩīĭįıǐȉȋ", 'i'),
        ("ĵ", 'j'),
        ("ķǩ", 'k'),
        ("ĺļľŀł", 'l'),
        ("ñńņňŉǹ", 'n'),
        ("òóôõöøōŏőǒǫǭǿȍȏȫȭȯȱ", 'o'),
        ("ŕŗřȑȓ", 'r'),
        ("śŝşšș", 's'),
        ("ţťŧț", 't'),
        ("ùúûüũūŭůűųǔǖǘǚǜȕȗ", 'u'),
        ("ŵ", 'w'),
        ("ýÿŷȳ", 'y'),
        ("źżž", 'z'),
    ];
    if ch.is_ascii() {
        return ch;
    }
    TABLE
        .iter()
        .find(|(accented, _)| accented.contains(ch))
        .map_or(ch, |(_, base)| *base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn card(term: &str, pron: &str, definition: &str) -> Card {
        let mut card = Card::new(
            1,
            term.into(),
            definition.into(),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        );
        card.pron = pron.into();
        card
    }

    fn score(query: &str, card: &Card) -> Option<Score> {
        Query::new(query).score(card)
    }

    #[test]
    fn the_term_outranks_the_prose() {
        let abash = card("Abash", "uh BASH", "to embarrass");
        let other = card("Abate", "uh BAYT", "to lessen; not to abash");
        assert!(score("abash", &abash) < score("abash", &other));
        assert!(score("aba", &abash).is_some());
        assert!(score("ABASH", &abash) == score("abash", &abash));
    }

    #[test]
    fn a_half_typed_word_matches_its_start() {
        let card = card("Circumlocution", "", "talking around a subject");
        assert_eq!(score("circum", &card).map(|s| s.tier), Some(PREFIX));
        assert_eq!(score("locution", &card).map(|s| s.tier), Some(INSIDE));
        assert_eq!(score("around", &card).map(|s| s.tier), Some(PROSE));
    }

    #[test]
    fn misspellings_still_find_the_word() {
        let card = card("Ubiquitous", "yoo BIK wih tus", "being everywhere");
        for typo in ["ubiquitus", "ubiqitous", "ubiquotous", "ubqiuitous"] {
            let found = score(typo, &card);
            assert!(found.is_some_and(Score::is_fuzzy), "{typo}: {found:?}");
        }
        // A swapped pair is one slip, not two.
        let abash = super::tests::card("Abash", "", "to embarrass");
        assert_eq!(score("abahs", &abash).map(|s| s.typos), Some(1));
        // Half-typed and misspelt at once.
        assert!(score("ubiqit", &card).is_some());
    }

    #[test]
    fn a_short_query_is_not_guessed_at() {
        let card = card("Abash", "", "to embarrass");
        assert!(score("abz", &card).is_none(), "three letters, no fuzz");
        assert!(score("zzzzzz", &card).is_none());
    }

    #[test]
    fn closer_spellings_rank_first() {
        let card = card("Separate", "", "to set apart");
        let one = score("seperate", &card).unwrap();
        let two = score("seperete", &card).unwrap();
        assert!(one < two);
    }

    #[test]
    fn accents_and_tones_are_ignored() {
        let naive = card("Naïve", "", "lacking experience");
        assert!(score("naive", &naive).is_some_and(|s| s.tier == EXACT));
        let hello = card("你好", "nǐ hǎo", "hello");
        assert_eq!(score("ni hao", &hello).map(|s| s.tier), Some(WORD_PREFIX));
        assert_eq!(score("nihao", &hello).map(|s| s.tier), Some(WORD_PREFIX));
        assert_eq!(score("hao", &hello).map(|s| s.tier), Some(WORD_PREFIX));
        assert_eq!(score("你", &hello).map(|s| s.tier), Some(PREFIX));
        assert_eq!(fold("Café"), "cafe");
        assert_eq!(fold("e\u{301}"), "e", "a combining accent folds away too");
    }

    #[test]
    fn near_misses_give_way_to_real_hits() {
        let cards = [card("term1", "", "one"), card("term2", "", "two")];
        let ranked = |query: &str| {
            let query = Query::new(query);
            rank(
                cards
                    .iter()
                    .enumerate()
                    .filter_map(|(i, c)| Some((query.score(c)?, i)))
                    .collect(),
            )
        };
        assert_eq!(
            ranked("term1"),
            vec![0],
            "term2 is one off, but term1 is here"
        );
        assert_eq!(ranked("terma"), vec![0, 1], "nothing exact: both are close");
    }

    #[test]
    fn prefix_distance_counts_edits_against_the_best_prefix() {
        let q = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(prefix_distance(&q("aba"), "abash"), 0);
        assert_eq!(prefix_distance(&q("abesh"), "abash"), 1);
        assert_eq!(prefix_distance(&q("abashh"), "abash"), 1);
        assert_eq!(prefix_distance(&q(""), "abash"), 0);
        assert_eq!(prefix_distance(&q("ab"), ""), 2);
    }
}
