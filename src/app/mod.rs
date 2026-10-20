//! Application state and the Vim-style key dispatcher.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{Duration, Local, NaiveDate};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::complete;
use crate::model::{Card, CardState, Db, Deck, Grade, Grading, MAX_LOG, ReviewLog, Study};
use crate::search::{self, Query};
use crate::store;
use crate::textinput::TextInput;
use crate::util::Rng;

mod browse;
mod commands;
mod editor;
mod find;
mod keys;
mod session;
mod stats;
#[cfg(test)]
mod tests;
mod undo;

/// How many cards must pass before a lapsed card comes back around.
const REQUEUE_GAP: usize = 8;

/// How many answers between mid-session saves.
const CHECKPOINT_EVERY: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Decks,
    Browse,
    Review,
    Stats,
    Help,
    /// One word from the browser, laid out in full to be read.
    Card,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Command,
    /// `/` — move the cursor to the best match as you type.
    Search,
    /// `f` — narrow the list to the matches as you type.
    Filter,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Command => "COMMAND",
            Self::Search => "SEARCH",
            Self::Filter => "FILTER",
        }
    }
}

/// The editable fields of a card, in tab order.
pub const FIELDS: [&str; 7] = [
    "Term",
    "Pron",
    "Pos",
    "Definition",
    "Example",
    "Notes",
    "Confusing",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditTarget {
    New { deck: u64 },
    Existing { deck: u64, card: u64 },
}

pub struct Editor {
    pub target: EditTarget,
    pub fields: Vec<TextInput>,
    pub field: usize,
}

impl Editor {
    fn blank(deck: u64) -> Self {
        Self {
            target: EditTarget::New { deck },
            fields: FIELDS.iter().map(|_| TextInput::default()).collect(),
            field: 0,
        }
    }

    fn from_card(deck: u64, card: &Card) -> Self {
        let values = [
            card.term.as_str(),
            card.pron.as_str(),
            card.pos.as_str(),
            card.definition.as_str(),
            card.example.as_str(),
            card.notes.as_str(),
            card.confusing.as_str(),
        ];
        Self {
            target: EditTarget::Existing {
                deck,
                card: card.id,
            },
            fields: values.iter().map(|v| TextInput::new(escape(v))).collect(),
            field: 0,
        }
    }

    fn deck(&self) -> u64 {
        match self.target {
            EditTarget::New { deck } | EditTarget::Existing { deck, .. } => deck,
        }
    }

    fn current_mut(&mut self) -> &mut TextInput {
        &mut self.fields[self.field]
    }
}

/// Multi-line fields are edited on one line, with newlines shown as `\n`.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n")
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Which cards a session draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Everything due today, plus the day's ration of new cards.
    Due,
    /// Like `Due`, with this many new cards on top of the day's ration — a
    /// one-off for a day you want to push further, leaving `new_per_day` be.
    More(u32),
    /// Words carrying a status, regardless of when they are next due.
    /// `None` means any status at all.
    Flagged(Option<Study>),
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Due => "due",
            Self::More(_) => "more",
            Self::Flagged(None) => "flagged",
            Self::Flagged(Some(status)) => status.label(),
        }
    }

    fn matches(self, card: &Card) -> bool {
        match self {
            Self::Due | Self::More(_) => true,
            Self::Flagged(None) => card.is_flagged(),
            Self::Flagged(Some(status)) => card.status == status,
        }
    }
}

/// `:marked` — what the browser is restricted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusFilter {
    #[default]
    Off,
    /// Any word carrying a status.
    Any,
    Only(Study),
}

impl StatusFilter {
    fn matches(self, card: &Card) -> bool {
        match self {
            Self::Off => true,
            Self::Any => card.is_flagged(),
            Self::Only(status) => card.status == status,
        }
    }

    pub fn is_on(self) -> bool {
        self != Self::Off
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "all",
            Self::Any => "flagged",
            Self::Only(status) => status.label(),
        }
    }
}

pub struct Session {
    /// The deck's id, not its position — see [`Db::deck_by_id`].
    pub deck: u64,
    pub scope: Scope,
    pub queue: VecDeque<u64>,
    pub revealed: bool,
    /// Answers given, retries included.
    pub answered: u32,
    pub lapses: u32,
    pub initial: usize,
    /// Cards seen off for the session: answered without being put back, or
    /// suspended out of the queue. Progress counts these, not answers — the
    /// same card answered three times is not three cards done.
    pub completed: usize,
    /// Cards put back for another go and not yet finished.
    pub retrying: HashSet<u64>,
}

impl Session {
    pub fn current(&self) -> Option<u64> {
        self.queue.front().copied()
    }
}

/// Enough state to put a graded card back the way it was.
struct LastAnswer {
    deck: u64,
    card: Card,
    requeued_at: Option<usize>,
    spent_new: bool,
    grade: Grade,
    /// Whether the card was already awaiting another go, so undo can put the
    /// retry count back exactly as it was.
    was_retrying: bool,
}

/// How far back `u` can go. Each step holds the collection as it was, so this
/// is a memory budget as much as a feature: a few tens of megabytes for a
/// collection of a couple of thousand cards.
const UNDO_DEPTH: usize = 64;

/// The collection as it stood before one change, with the cursor that was on
/// screen at the time. Undo is snapshot-based rather than a list of inverse
/// operations: every change is undoable the day it is written, and no edit can
/// quietly grow a hole in the history by forgetting to describe its opposite.
struct Step {
    db: Db,
    /// What the change did, as `u` and `Ctrl-r` report it: "deleting Abash".
    label: String,
    /// Where it happened, so undo takes you back to it instead of leaving you
    /// wherever you had wandered since.
    screen: Screen,
    deck: u64,
    row: usize,
}

/// A Tab completion in progress on the command line.
pub struct Completion {
    /// Character index where the word being completed starts.
    at: usize,
    pub candidates: Vec<String>,
    /// Which candidate is showing, once Tab has started cycling through them.
    pub index: Option<usize>,
}

pub enum ConfirmAction {
    DeleteDeck(u64),
    ForgetDeck(u64),
    DiscardAndQuit,
    /// Replace the live collection with a kept copy.
    Restore(PathBuf),
}

pub struct Confirm {
    pub prompt: String,
    pub action: ConfirmAction,
}

#[derive(Default)]
pub struct Status {
    pub text: String,
    pub error: bool,
}

pub struct App {
    pub db: Db,
    /// Where this collection is written. Held explicitly rather than looked
    /// up globally, so tests never touch the real one.
    pub path: PathBuf,
    pub today: NaiveDate,
    pub screen: Screen,
    pub mode: Mode,

    /// Cursor position in the deck list. A position, so it is clamped rather
    /// than looked up when decks come and go.
    pub decks_sel: usize,
    /// First deck row on screen. Kept between frames, or every redraw would
    /// start at the top and drag the cursor to the bottom of the window.
    pub decks_off: usize,
    /// Id of the deck the browser is showing; `0` when there is none.
    pub browse_deck: u64,
    pub browse_sel: usize,
    /// First card row on screen, as `decks_off` is for the deck list.
    pub browse_off: usize,
    /// Indices into the browsed deck's cards, after filtering.
    pub rows: Vec<usize>,
    pub filter: String,
    /// The last `/` search: what `n` and `N` step through, and what the
    /// browser picks out. The list itself is left whole.
    pub search: String,
    /// Where the cursor, the filter and the search were when `/` or `f`
    /// opened, so `Esc` can put them back.
    line_origin: Option<(usize, String)>,
    /// `:marked` — restrict the browser to words carrying a status.
    pub status_filter: StatusFilter,
    /// `z` — cover every meaning in the browser, so the word list reads as a
    /// test rather than as an answer sheet.
    pub hide_meanings: bool,
    /// `space` — uncover the word under the cursor, until the cursor moves.
    pub peek: bool,

    pub editor: Option<Editor>,
    pub session: Option<Session>,
    pub confirm: Option<Confirm>,

    /// Backing store for `:` and `/`.
    pub line: TextInput,
    /// Tab completion offered for what is typed on the command line.
    pub completion: Option<Completion>,
    pub status: Status,
    /// A half-typed operator such as `g` or `d`.
    pub pending: Option<char>,
    pub count: String,

    pub help_scroll: u16,
    pub help_from: Screen,
    /// When this copy of the collection was last written, so a save can tell
    /// whether something else has written since.
    written: Option<std::time::SystemTime>,
    /// How far the review card is scrolled. Reset whenever the card, or which
    /// side of it is showing, changes.
    pub card_scroll: u16,

    last_answer: Option<LastAnswer>,
    /// Changes already made, newest last; `u` walks back along it. Answering a
    /// card is not on here — a review session has its own, finer undo, which
    /// knows about the queue as well as the card.
    undo: Vec<Step>,
    /// Changes `u` has taken back, newest last; `Ctrl-r` walks forward again.
    /// Cleared by the next change, as in Vim: the future stops being the
    /// future once you have done something else instead.
    redo: Vec<Step>,

    pub dirty: bool,
    pub quit: bool,
    rng: Rng,
}

impl App {
    pub fn new(db: Db, path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let mut app = Self {
            db,
            written: store::modified(&path),
            path,
            today: Local::now().date_naive(),
            screen: Screen::Decks,
            mode: Mode::Normal,
            decks_sel: 0,
            decks_off: 0,
            browse_deck: 0,
            browse_sel: 0,
            browse_off: 0,
            rows: Vec::new(),
            filter: String::new(),
            search: String::new(),
            line_origin: None,
            status_filter: StatusFilter::default(),
            hide_meanings: false,
            peek: false,
            editor: None,
            session: None,
            confirm: None,
            line: TextInput::default(),
            completion: None,
            status: Status::default(),
            pending: None,
            count: String::new(),
            help_scroll: 0,
            help_from: Screen::Decks,
            card_scroll: 0,
            last_answer: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            quit: false,
            rng: Rng::from_entropy(),
        };
        if app.db.decks.is_empty() {
            app.info("no decks yet — press :import <file.csv> or a to add a card");
        }
        app
    }

    /// Roll the clock over if the program has been left open past midnight,
    /// so due counts and the new-card allowance do not go stale.
    pub fn refresh_today(&mut self) {
        let now = Local::now().date_naive();
        if now != self.today {
            self.today = now;
            self.refresh_rows();
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.status = Status {
            text: text.into(),
            error: false,
        };
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.status = Status {
            text: text.into(),
            error: true,
        };
    }

    /// Id of the deck the current screen acts on.
    pub fn active_deck(&self) -> Option<u64> {
        let id = match self.screen {
            Screen::Browse | Screen::Card => self.browse_deck,
            Screen::Review => self.session.as_ref()?.deck,
            _ => self.db.decks.get(self.decks_sel)?.id,
        };
        self.db.deck_by_id(id).map(|d| d.id)
    }

    /// `(deck id, card position)` for the browser's selected row.
    pub fn selected_card(&self) -> Option<(u64, usize)> {
        let index = *self.rows.get(self.browse_sel)?;
        Some((self.browse_deck, index))
    }

    /// Recompute the filtered row list for the browser. A filter lists its
    /// matches best first — the word you meant ahead of the sentence that
    /// mentions it — and, with nothing typed, the deck keeps its own order.
    pub fn refresh_rows(&mut self) {
        let query = Query::new(&self.filter);
        self.rows = match self.db.deck_by_id(self.browse_deck) {
            None => Vec::new(),
            Some(deck) => {
                let shown = deck
                    .cards
                    .iter()
                    .enumerate()
                    .filter(|(_, card)| self.status_filter.matches(card));
                if query.is_empty() {
                    shown.map(|(i, _)| i).collect()
                } else {
                    search::rank(
                        shown
                            .filter_map(|(i, card)| query.score(card).map(|score| (score, i)))
                            .collect(),
                    )
                }
            }
        };
        if self.browse_sel >= self.rows.len() {
            self.browse_sel = self.rows.len().saturating_sub(1);
        }
    }

    /// Write the collection. Returns where the previous contents were kept
    /// when they turned out to be someone else's newer version.
    pub fn save(&mut self) -> Result<Option<PathBuf>> {
        // Another gram — or `gram import` in a second terminal — may have
        // written since this copy was loaded. Ours still wins: it is the one
        // being studied. But theirs is kept rather than lost.
        let on_disk = store::modified(&self.path);
        let conflict = if on_disk != self.written && on_disk.is_some() {
            // If their version cannot be kept, do not overwrite it. A failed
            // save leaves this session intact and recoverable; a save that
            // clobbers an unread collection does not.
            Some(store::keep_conflicting_copy(&self.path).with_context(|| {
                format!(
                    "{} changed on disk and that version could not be backed up, \
                     so it was left alone",
                    self.path.display()
                )
            })?)
        } else {
            None
        };

        store::save_to(&self.db, &self.path)?;
        self.written = store::modified(&self.path);
        self.dirty = false;
        Ok(conflict)
    }

    fn autosave(&mut self) {
        match self.save() {
            Ok(None) => {}
            Ok(Some(aside)) => self.error(format!(
                "changed on disk — their version kept as {} (:restore)",
                store::backup_label(&aside)
            )),
            Err(err) => self.error(format!("save failed: {err:#}")),
        }
    }
}
