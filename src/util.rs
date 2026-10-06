//! Small helpers: a seeded shuffle, interval formatting, and the text
//! measuring every layout in [`crate::ui`] depends on.

use std::time::{SystemTime, UNIX_EPOCH};

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// xorshift64* — enough randomness to shuffle a study queue, no dependency.
pub struct Rng(u64);

impl Rng {
    pub fn from_entropy() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x2545_F491_4F6C_DD1D);
        Self::seeded(nanos ^ (std::process::id() as u64) << 32)
    }

    pub fn seeded(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next_u64() % bound as u64) as usize
        }
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

/// `0` -> `now`, `1` -> `1d`, `45` -> `1.5mo`, `400` -> `1.1y`.
pub fn humanize_days(days: u32) -> String {
    match days {
        0 => "now".to_string(),
        1 => "1d".to_string(),
        2..=30 => format!("{days}d"),
        31..=364 => {
            let months = days as f32 / 30.4;
            if months < 10.0 {
                format!("{months:.1}mo")
            } else {
                format!("{months:.0}mo")
            }
        }
        _ => {
            let years = days as f32 / 365.0;
            if years < 10.0 {
                format!("{years:.1}y")
            } else {
                format!("{years:.0}y")
            }
        }
    }
}

/// Split a string on `*emphasis*` markers into `(text, emphasised)` runs.
///
/// The Word Smart importer wraps the target word in asterisks so it can be
/// highlighted — or hidden — when a sentence is shown.
pub fn split_emphasis(text: &str) -> Vec<(&str, bool)> {
    let mut runs = Vec::new();
    let mut rest = text;
    let mut emphasised = false;
    while let Some(at) = rest.find('*') {
        if at > 0 {
            runs.push((&rest[..at], emphasised));
        }
        rest = &rest[at + 1..];
        emphasised = !emphasised;
    }
    if !rest.is_empty() {
        runs.push((rest, emphasised));
    }
    runs
}

// -- measuring text -------------------------------------------------------
//
// A terminal lays out columns, not characters. `한` takes two of them, a
// combining accent takes none, and counting `chars()` instead gets tables,
// truncation and the cursor wrong by exactly that difference.

/// Columns `text` occupies on screen.
pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// The text as user-perceived characters: `e` + `◌́` is one, not two.
pub fn clusters(text: &str) -> impl Iterator<Item = &str> {
    text.graphemes(true)
}

/// How many clusters `text` is.
pub fn cluster_count(text: &str) -> usize {
    clusters(text).count()
}

/// Byte offset where cluster `index` starts, or the length when past the end.
pub fn cluster_offset(text: &str, index: usize) -> usize {
    text.grapheme_indices(true)
        .nth(index)
        .map(|(offset, _)| offset)
        .unwrap_or(text.len())
}

/// Truncate to `columns`, adding an ellipsis when it cuts. Never splits a
/// double-width character down the middle.
pub fn truncate(text: &str, columns: usize) -> String {
    if width(text) <= columns {
        return text.to_string();
    }
    if columns <= 1 {
        return "…".to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    // One column is kept back for the ellipsis.
    for cluster in clusters(text) {
        let next = width(cluster);
        if used + next > columns - 1 {
            break;
        }
        out.push_str(cluster);
        used += next;
    }
    out.push('…');
    out
}

/// Truncate to `columns`, then pad with spaces to exactly that many — what a
/// table column needs, and what `{:<width$}` cannot do.
pub fn pad(text: &str, columns: usize) -> String {
    let mut out = truncate(text, columns);
    let padding = columns.saturating_sub(width(&out));
    out.extend(std::iter::repeat_n(' ', padding));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Korean, a combining accent, and an emoji: two columns, zero, two.
    #[test]
    fn width_counts_columns_not_characters() {
        assert_eq!(width("abash"), 5);
        assert_eq!(width("한국어"), 6, "three wide characters");
        assert_eq!(width("e\u{301}"), 1, "e plus a combining acute");
        assert_eq!(width("naïveté"), 7);
    }

    #[test]
    fn clusters_hold_combining_marks_together() {
        assert_eq!(cluster_count("e\u{301}te\u{301}"), 3, "é t é, five chars");
        assert_eq!(cluster_count("한국어"), 3);
        let text = "e\u{301}x";
        assert_eq!(cluster_offset(text, 1), 3, "past e and its accent");
        assert_eq!(cluster_offset(text, 9), text.len(), "past the end");
    }

    #[test]
    fn truncate_measures_in_columns() {
        assert_eq!(truncate("abash", 10), "abash");
        assert_eq!(truncate("circumlocution", 8), "circuml…");
        assert_eq!(truncate("abc", 1), "…");

        // Three wide characters are six columns, so five columns fit one of
        // them plus the ellipsis — not two characters and an overflow.
        assert_eq!(truncate("한국어", 6), "한국어");
        assert_eq!(truncate("한국어", 5), "한국…");
        assert_eq!(width(&truncate("한국어", 5)), 5);
        assert_eq!(truncate("한국어", 4), "한…");
        assert_eq!(width(&truncate("한국어", 4)), 3, "never splits one in half");
    }

    #[test]
    fn pad_fills_to_the_column() {
        assert_eq!(pad("abash", 8), "abash   ");
        assert_eq!(width(&pad("한국어", 8)), 8, "wide text pads less");
        assert_eq!(width(&pad("e\u{301}", 4)), 4, "and zero-width text more");
        assert_eq!(width(&pad("circumlocution", 8)), 8, "long text is cut");
    }

    #[test]
    fn shuffle_preserves_every_element() {
        let mut rng = Rng::seeded(42);
        let mut items: Vec<u32> = (0..100).collect();
        rng.shuffle(&mut items);
        items.sort_unstable();
        assert_eq!(items, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn shuffle_actually_reorders() {
        let mut rng = Rng::seeded(7);
        let original: Vec<u32> = (0..64).collect();
        let mut items = original.clone();
        rng.shuffle(&mut items);
        assert_ne!(items, original);
    }

    #[test]
    fn below_stays_in_range() {
        let mut rng = Rng::seeded(1);
        for _ in 0..1000 {
            assert!(rng.below(10) < 10);
        }
        assert_eq!(rng.below(0), 0);
    }

    #[test]
    fn humanizes_intervals() {
        assert_eq!(humanize_days(0), "now");
        assert_eq!(humanize_days(1), "1d");
        assert_eq!(humanize_days(6), "6d");
        assert_eq!(humanize_days(45), "1.5mo");
        assert_eq!(humanize_days(400), "1.1y");
    }

    #[test]
    fn splits_emphasis_runs() {
        let runs = split_emphasis("the agony *abated* today");
        assert_eq!(
            runs,
            vec![("the agony ", false), ("abated", true), (" today", false)]
        );
    }

    #[test]
    fn handles_text_without_markers() {
        assert_eq!(split_emphasis("plain"), vec![("plain", false)]);
        assert_eq!(split_emphasis(""), Vec::new());
    }
}
