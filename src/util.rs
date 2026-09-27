//! Small helpers: a seeded shuffle and interval formatting.

use std::time::{SystemTime, UNIX_EPOCH};

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
        Self(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
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

#[cfg(test)]
mod tests {
    use super::*;

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
