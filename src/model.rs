//! Cards, decks, and the SM-2 scheduler.

use chrono::{Duration, NaiveDate};
use serde::{Deserialize, Serialize};

pub const DEFAULT_EASE: f32 = 2.5;
pub const MIN_EASE: f32 = 1.3;
pub const MAX_INTERVAL: u32 = 36_500;

fn default_ease() -> f32 {
    DEFAULT_EASE
}

fn default_new_per_day() -> u32 {
    20
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardState {
    /// Never answered.
    #[default]
    New,
    /// Answered `Again` at least once and not yet back on the review track.
    Learning,
    /// On the spaced-repetition track.
    Review,
}

impl CardState {
    pub fn label(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Learning => "learn",
            Self::Review => "review",
        }
    }
}

/// How answers are collected during review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Grading {
    /// Two buttons: "I know this" and "I don't know".
    #[default]
    Binary,
    /// The full Again / Hard / Good / Easy row.
    Sm2,
}

impl Grading {
    pub fn label(self) -> &'static str {
        match self {
            Self::Binary => "binary",
            Self::Sm2 => "sm2",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "binary" | "bin" | "2" => Some(Self::Binary),
            "sm2" | "four" | "4" => Some(Self::Sm2),
            _ => None,
        }
    }
}

/// The four answer buttons, in SM-2 order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    Again,
    Hard,
    Good,
    Easy,
}

impl Grade {
    pub const ALL: [Grade; 4] = [Grade::Again, Grade::Hard, Grade::Good, Grade::Easy];

    pub fn label(self) -> &'static str {
        match self {
            Self::Again => "Again",
            Self::Hard => "Hard",
            Self::Good => "Good",
            Self::Easy => "Easy",
        }
    }

    pub fn key(self) -> char {
        match self {
            Self::Again => '1',
            Self::Hard => '2',
            Self::Good => '3',
            Self::Easy => '4',
        }
    }

    pub fn from_index(i: u8) -> Option<Self> {
        Self::ALL.get(i as usize).copied()
    }

    pub fn index(self) -> u8 {
        match self {
            Self::Again => 0,
            Self::Hard => 1,
            Self::Good => 2,
            Self::Easy => 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: u64,
    pub term: String,
    /// Pronunciation respelling, e.g. `uh BASH`.
    #[serde(default)]
    pub pron: String,
    /// Part of speech: `n`, `v`, `adj`, ...
    #[serde(default)]
    pub pos: String,
    pub definition: String,
    /// Example sentences, one per line. The target word is wrapped in `*`.
    #[serde(default)]
    pub example: String,
    #[serde(default)]
    pub notes: String,

    // -- scheduling state --
    #[serde(default = "default_ease")]
    pub ease: f32,
    /// Current interval in days.
    #[serde(default)]
    pub interval: u32,
    /// Successful reviews so far.
    #[serde(default)]
    pub reps: u32,
    #[serde(default)]
    pub lapses: u32,
    #[serde(default)]
    pub state: CardState,
    pub due: NaiveDate,
    #[serde(default)]
    pub last_reviewed: Option<NaiveDate>,
    #[serde(default)]
    pub suspended: bool,
    /// In the user's "words I have to study" pile.
    #[serde(default)]
    pub marked: bool,
}

impl Card {
    pub fn new(id: u64, term: String, definition: String, today: NaiveDate) -> Self {
        Self {
            id,
            term,
            pron: String::new(),
            pos: String::new(),
            definition,
            example: String::new(),
            notes: String::new(),
            ease: DEFAULT_EASE,
            interval: 0,
            reps: 0,
            lapses: 0,
            state: CardState::New,
            due: today,
            last_reviewed: None,
            suspended: false,
            marked: false,
        }
    }

    pub fn is_due(&self, today: NaiveDate) -> bool {
        !self.suspended && self.state != CardState::New && self.due <= today
    }

    pub fn examples(&self) -> impl Iterator<Item = &str> {
        self.example.lines().filter(|l| !l.trim().is_empty())
    }

    /// Reset all scheduling state, keeping the content.
    pub fn forget(&mut self, today: NaiveDate) {
        self.ease = DEFAULT_EASE;
        self.interval = 0;
        self.reps = 0;
        self.lapses = 0;
        self.state = CardState::New;
        self.due = today;
        self.last_reviewed = None;
    }

    /// Apply an answer. Returns `true` when the card should be shown again
    /// later in the same session.
    pub fn grade(&mut self, grade: Grade, today: NaiveDate) -> bool {
        let prev = self.interval.max(1) as f32;

        match grade {
            Grade::Again => {
                if self.state == CardState::Review {
                    self.lapses += 1;
                }
                self.ease = (self.ease - 0.20).max(MIN_EASE);
                self.interval = 0;
                self.state = CardState::Learning;
                self.due = today;
                self.last_reviewed = Some(today);
                return true;
            }
            Grade::Hard => {
                self.ease = (self.ease - 0.15).max(MIN_EASE);
                self.interval = match self.state {
                    CardState::New | CardState::Learning => 1,
                    CardState::Review => grow(prev * 1.2, self.interval),
                };
            }
            Grade::Good => {
                self.interval = match self.state {
                    CardState::New => 1,
                    // Rebuilding after a lapse: start small and let it regrow.
                    CardState::Learning => grow(prev * self.ease * 0.5, 0),
                    CardState::Review if self.reps <= 1 => grow(6.0, self.interval),
                    CardState::Review => grow(prev * self.ease, self.interval),
                };
            }
            Grade::Easy => {
                self.ease += 0.15;
                self.interval = match self.state {
                    CardState::New => 4,
                    CardState::Learning => grow(prev * self.ease, 1),
                    CardState::Review => grow(prev * self.ease * 1.3, self.interval),
                };
            }
        }

        self.reps += 1;
        self.state = CardState::Review;
        self.last_reviewed = Some(today);
        self.due = today + Duration::days(self.interval as i64);
        false
    }

    /// Interval in days each button would produce, for the answer bar.
    /// `None` means "again in this session".
    pub fn preview(&self, today: NaiveDate) -> [Option<u32>; 4] {
        let mut out = [None; 4];
        for (i, grade) in Grade::ALL.iter().enumerate() {
            let mut probe = self.clone();
            if !probe.grade(*grade, today) {
                out[i] = Some(probe.interval);
            }
        }
        out
    }
}

/// Round an interval up, keeping it strictly larger than the previous one.
fn grow(value: f32, floor: u32) -> u32 {
    let days = value.round().max(1.0) as u32;
    days.max(floor.saturating_add(1)).min(MAX_INTERVAL)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deck {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub cards: Vec<Card>,
    #[serde(default = "default_new_per_day")]
    pub new_per_day: u32,
    /// Date the new-card allowance was last spent, and how much of it.
    #[serde(default)]
    pub intro_date: Option<NaiveDate>,
    #[serde(default)]
    pub intro_count: u32,
}

impl Deck {
    pub fn new(id: u64, name: String) -> Self {
        Self {
            id,
            name,
            cards: Vec::new(),
            new_per_day: default_new_per_day(),
            intro_date: None,
            intro_count: 0,
        }
    }

    pub fn card(&self, id: u64) -> Option<&Card> {
        self.cards.iter().find(|c| c.id == id)
    }

    pub fn card_mut(&mut self, id: u64) -> Option<&mut Card> {
        self.cards.iter_mut().find(|c| c.id == id)
    }

    /// How many unseen cards may still be introduced today.
    pub fn new_allowance(&self, today: NaiveDate) -> u32 {
        let spent = match self.intro_date {
            Some(date) if date == today => self.intro_count,
            _ => 0,
        };
        self.new_per_day.saturating_sub(spent)
    }

    pub fn spend_new(&mut self, today: NaiveDate, count: u32) {
        if self.intro_date != Some(today) {
            self.intro_date = Some(today);
            self.intro_count = 0;
        }
        self.intro_count += count;
    }

    pub fn counts(&self, today: NaiveDate) -> DeckCounts {
        let mut counts = DeckCounts::default();
        counts.total = self.cards.len();
        for card in &self.cards {
            if card.suspended {
                counts.suspended += 1;
                continue;
            }
            if card.marked {
                counts.marked += 1;
            }
            match card.state {
                CardState::New => counts.new += 1,
                CardState::Learning if card.due <= today => counts.learning += 1,
                CardState::Review if card.due <= today => counts.due += 1,
                _ => {}
            }
        }
        counts.new = counts.new.min(self.new_allowance(today) as usize);
        counts
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DeckCounts {
    pub total: usize,
    pub new: usize,
    pub learning: usize,
    pub due: usize,
    pub suspended: usize,
    pub marked: usize,
}

impl DeckCounts {
    pub fn pending(&self) -> usize {
        self.new + self.learning + self.due
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewLog {
    pub date: NaiveDate,
    pub deck_id: u64,
    pub card_id: u64,
    /// 0..=3, matching `Grade::index`.
    pub grade: u8,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Db {
    #[serde(default)]
    pub decks: Vec<Deck>,
    /// How the answer bar is presented. A collection-wide preference.
    #[serde(default)]
    pub grading: Grading,
    #[serde(default)]
    pub log: Vec<ReviewLog>,
    #[serde(default)]
    next_card_id: u64,
    #[serde(default)]
    next_deck_id: u64,
}

impl Db {
    pub fn next_card_id(&mut self) -> u64 {
        self.next_card_id += 1;
        self.next_card_id
    }

    pub fn next_deck_id(&mut self) -> u64 {
        self.next_deck_id += 1;
        self.next_deck_id
    }

    pub fn deck_index(&self, name: &str) -> Option<usize> {
        self.decks
            .iter()
            .position(|d| d.name.eq_ignore_ascii_case(name))
    }

    pub fn add_deck(&mut self, name: String) -> usize {
        let id = self.next_deck_id();
        self.decks.push(Deck::new(id, name));
        self.decks.len() - 1
    }

    /// Repair id counters after loading a file that was edited by hand.
    pub fn reseat_ids(&mut self) {
        let max_card = self
            .decks
            .iter()
            .flat_map(|d| d.cards.iter())
            .map(|c| c.id)
            .max()
            .unwrap_or(0);
        let max_deck = self.decks.iter().map(|d| d.id).max().unwrap_or(0);
        self.next_card_id = self.next_card_id.max(max_card);
        self.next_deck_id = self.next_deck_id.max(max_deck);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
    }

    fn card() -> Card {
        Card::new(1, "abash".into(), "to embarrass".into(), today())
    }

    #[test]
    fn new_card_good_then_good_follows_sm2() {
        let mut c = card();
        assert!(!c.grade(Grade::Good, today()));
        assert_eq!(c.interval, 1);
        assert_eq!(c.state, CardState::Review);

        assert!(!c.grade(Grade::Good, today()));
        assert_eq!(c.interval, 6);

        c.grade(Grade::Good, today());
        assert_eq!(c.interval, 15); // 6 * 2.5
    }

    #[test]
    fn again_requeues_and_drops_ease() {
        let mut c = card();
        c.grade(Grade::Good, today());
        c.grade(Grade::Good, today());
        let ease = c.ease;

        assert!(c.grade(Grade::Again, today()));
        assert_eq!(c.state, CardState::Learning);
        assert_eq!(c.interval, 0);
        assert_eq!(c.lapses, 1);
        assert!(c.ease < ease);
    }

    #[test]
    fn ease_never_falls_below_floor() {
        let mut c = card();
        for _ in 0..40 {
            c.grade(Grade::Again, today());
        }
        assert!(c.ease >= MIN_EASE);
    }

    #[test]
    fn intervals_always_advance() {
        let mut c = card();
        c.grade(Grade::Good, today());
        for _ in 0..20 {
            let before = c.interval;
            c.grade(Grade::Hard, today());
            assert!(c.interval > before, "{} !> {}", c.interval, before);
        }
    }

    #[test]
    fn preview_matches_actual_grade() {
        let mut c = card();
        c.grade(Grade::Good, today());
        let preview = c.preview(today());

        let mut probe = c.clone();
        probe.grade(Grade::Easy, today());
        assert_eq!(preview[3], Some(probe.interval));
        assert_eq!(preview[0], None); // Again stays in-session
    }

    #[test]
    fn new_allowance_resets_across_days() {
        let mut deck = Deck::new(1, "test".into());
        deck.new_per_day = 5;
        deck.spend_new(today(), 5);
        assert_eq!(deck.new_allowance(today()), 0);

        let tomorrow = today() + Duration::days(1);
        assert_eq!(deck.new_allowance(tomorrow), 5);
    }
}
