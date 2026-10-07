//! Application state and the Vim-style key dispatcher.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{Duration, Local, NaiveDate};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::complete;
use crate::model::{Card, CardState, Db, Deck, Grade, Grading, MAX_LOG, ReviewLog, Study};
use crate::store;
use crate::textinput::TextInput;
use crate::util::Rng;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Command,
    Search,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Command => "COMMAND",
            Self::Search => "SEARCH",
        }
    }
}

/// The six editable fields of a card, in tab order.
pub const FIELDS: [&str; 6] = ["Term", "Pron", "Pos", "Definition", "Example", "Notes"];

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
    /// Words carrying a status, regardless of when they are next due.
    /// `None` means any status at all.
    Flagged(Option<Study>),
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Due => "due",
            Self::Flagged(None) => "flagged",
            Self::Flagged(Some(status)) => status.label(),
        }
    }

    fn matches(self, card: &Card) -> bool {
        match self {
            Self::Due => true,
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

    // -- messaging -------------------------------------------------------

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

    // -- selection helpers -----------------------------------------------

    /// Id of the deck the current screen acts on.
    pub fn active_deck(&self) -> Option<u64> {
        let id = match self.screen {
            Screen::Browse => self.browse_deck,
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

    /// Recompute the filtered row list for the browser.
    pub fn refresh_rows(&mut self) {
        let needle = self.filter.to_lowercase();
        self.rows = match self.db.deck_by_id(self.browse_deck) {
            None => Vec::new(),
            Some(deck) => deck
                .cards
                .iter()
                .enumerate()
                .filter(|(_, card)| self.status_filter.matches(card))
                .filter(|(_, card)| {
                    needle.is_empty()
                        || card.term.to_lowercase().contains(&needle)
                        || card.definition.to_lowercase().contains(&needle)
                        || card.notes.to_lowercase().contains(&needle)
                })
                .map(|(i, _)| i)
                .collect(),
        };
        if self.browse_sel >= self.rows.len() {
            self.browse_sel = self.rows.len().saturating_sub(1);
        }
    }

    // -- study sessions ---------------------------------------------------

    fn build_queue(&mut self, deck_id: u64, scope: Scope) -> VecDeque<u64> {
        let Some(deck) = self.db.deck_by_id(deck_id) else {
            return VecDeque::new();
        };
        let today = self.today;

        // A status drill ignores due dates: the whole point is to pull those
        // words forward and hammer them.
        if let Scope::Flagged(_) = scope {
            let mut flagged: Vec<u64> = deck
                .cards
                .iter()
                .filter(|c| !c.suspended && scope.matches(c))
                .map(|c| c.id)
                .collect();
            self.rng.shuffle(&mut flagged);
            return flagged.into();
        }

        let mut due: Vec<u64> = deck
            .cards
            .iter()
            .filter(|c| c.is_due(today))
            .map(|c| c.id)
            .collect();
        let allowance = deck.new_allowance(today) as usize;
        let mut fresh: Vec<u64> = deck
            .cards
            .iter()
            .filter(|c| !c.suspended && c.state == CardState::New)
            .map(|c| c.id)
            .take(allowance)
            .collect();

        self.rng.shuffle(&mut due);
        self.rng.shuffle(&mut fresh);

        // Spread the new cards evenly through the review backlog rather than
        // front-loading them.
        let total = due.len() + fresh.len();
        let mut queue = due;
        if !fresh.is_empty() {
            let step = total as f32 / fresh.len() as f32;
            for (i, id) in fresh.into_iter().enumerate() {
                let at = (((i as f32) + 0.5) * step).round() as usize;
                queue.insert(at.min(queue.len()), id);
            }
        }
        queue.into()
    }

    pub fn start_session(&mut self, deck_id: u64, scope: Scope) {
        let queue = self.build_queue(deck_id, scope);
        if queue.is_empty() {
            let name = self
                .db
                .deck_by_id(deck_id)
                .map(|d| d.name.clone())
                .unwrap_or_default();
            self.info(match scope {
                Scope::Due => format!("{name}: nothing due — come back tomorrow"),
                Scope::Flagged(None) => {
                    format!("{name}: no words have a status — press m to set one")
                }
                Scope::Flagged(Some(status)) => {
                    format!("{name}: no {} words — press m to set one", status.label())
                }
            });
            return;
        }
        let initial = queue.len();
        self.session = Some(Session {
            deck: deck_id,
            scope,
            queue,
            revealed: false,
            answered: 0,
            lapses: 0,
            initial,
            completed: 0,
            retrying: HashSet::new(),
        });
        self.last_answer = None;
        self.card_scroll = 0;
        self.screen = Screen::Review;
        self.info(match scope {
            Scope::Due => format!("studying {initial} cards — space to reveal"),
            Scope::Flagged(_) => format!(
                "drilling {initial} {} words — space to reveal",
                scope.label()
            ),
        });
    }

    fn answer(&mut self, grade: Grade) {
        let today = self.today;
        let binary = self.db.grading == Grading::Binary;
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let Some(card_id) = session.queue.pop_front() else {
            return;
        };
        let deck_id = session.deck;
        let drill = matches!(session.scope, Scope::Flagged(_));

        let Some(deck) = self.db.deck_by_id_mut(deck_id) else {
            return;
        };
        let Some(card) = deck.card_mut(card_id) else {
            return;
        };

        let snapshot = card.clone();
        let was_new = card.state == CardState::New;
        let requeue = card.grade(grade, today);

        // "I don't know" files the word under things to study — but never
        // overwrites a status you set deliberately. The snapshot above still
        // holds the old value, so undo restores it.
        if binary && grade == Grade::Again && card.status == Study::None {
            card.status = Study::Marked;
        }

        if was_new {
            deck.spend_new(today, 1);
        }

        self.db.log.push(ReviewLog {
            date: today,
            deck_id,
            card_id,
            grade: grade.index(),
            drill,
            learning: was_new,
        });
        // Keep the log — and so the collection file — bounded.
        if self.db.log.len() > MAX_LOG {
            let excess = self.db.log.len() - MAX_LOG;
            self.db.log.drain(..excess);
        }

        let session = self.session.as_mut().expect("session checked above");
        session.answered += 1;
        let was_retrying = session.retrying.contains(&card_id);
        let requeued_at = if requeue {
            session.lapses += 1;
            session.retrying.insert(card_id);
            let at = REQUEUE_GAP.min(session.queue.len());
            session.queue.insert(at, card_id);
            Some(at)
        } else {
            session.retrying.remove(&card_id);
            session.completed += 1;
            None
        };
        session.revealed = false;

        self.last_answer = Some(LastAnswer {
            deck: deck_id,
            card: snapshot,
            requeued_at,
            spent_new: was_new,
            grade,
            was_retrying,
        });
        self.dirty = true;
        self.card_scroll = 0;

        if self.session.as_ref().is_some_and(|s| s.queue.is_empty()) {
            self.finish_session();
            return;
        }
        // Checkpoint as you go: a crash, a closed terminal or a flat battery
        // mid-session should cost a handful of answers, not all of them.
        if self
            .session
            .as_ref()
            .is_some_and(|s| s.answered % CHECKPOINT_EVERY == 0)
        {
            self.autosave();
        }
    }

    fn finish_session(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        let name = self
            .db
            .deck_by_id(session.deck)
            .map(|d| d.name.clone())
            .unwrap_or_default();
        self.screen = Screen::Decks;
        if let Some(index) = self.db.index_of(session.deck) {
            self.decks_sel = index;
        }
        let accuracy = if session.answered > 0 {
            100.0 * (1.0 - session.lapses as f32 / session.answered as f32)
        } else {
            0.0
        };
        self.info(format!(
            "{name} ({}): {} answers, {:.0}% correct — session complete",
            session.scope.label(),
            session.answered,
            accuracy
        ));
        self.autosave();
    }

    fn undo_answer(&mut self) {
        let Some(last) = self.last_answer.take() else {
            self.error("nothing to undo");
            return;
        };
        let card_id = last.card.id;

        if let Some(deck) = self.db.deck_by_id_mut(last.deck) {
            if last.spent_new {
                deck.intro_count = deck.intro_count.saturating_sub(1);
            }
            if let Some(card) = deck.card_mut(card_id) {
                *card = last.card;
            }
        }
        // Drop the log entry this answer produced.
        if self
            .db
            .log
            .last()
            .is_some_and(|entry| entry.card_id == card_id)
        {
            self.db.log.pop();
        }

        if let Some(session) = self.session.as_mut() {
            if let Some(at) = last.requeued_at {
                if session.queue.get(at) == Some(&card_id) {
                    session.queue.remove(at);
                }
                session.lapses = session.lapses.saturating_sub(1);
            } else {
                session.completed = session.completed.saturating_sub(1);
            }
            // Put the retry count back where the answer found it.
            if last.was_retrying {
                session.retrying.insert(card_id);
            } else {
                session.retrying.remove(&card_id);
            }
            session.queue.push_front(card_id);
            session.answered = session.answered.saturating_sub(1);
            session.revealed = true;
        }
        self.dirty = true;
        self.card_scroll = 0;
        self.info(format!("undid {}", last.grade.label().to_lowercase()));
    }

    // -- persistence -------------------------------------------------------

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

    // -- card editing -------------------------------------------------------

    fn open_new_card(&mut self) {
        let Some(deck) = self.active_deck() else {
            self.error("create a deck first: :deck <name>");
            return;
        };
        self.editor = Some(Editor::blank(deck));
        self.mode = Mode::Insert;
        self.info("new card — Tab/j/k switch fields, ZZ saves, q cancels");
    }

    fn open_edit_card(&mut self) {
        let Some((deck_id, card_index)) = self.selected_card() else {
            self.error("no card selected");
            return;
        };
        let Some(card) = self
            .db
            .deck_by_id(deck_id)
            .and_then(|d| d.cards.get(card_index))
        else {
            self.error("no card selected");
            return;
        };
        self.editor = Some(Editor::from_card(deck_id, card));
        self.mode = Mode::Normal;
        self.info("editing — i to insert, ZZ saves, q cancels");
    }

    fn edit_review_card(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let (deck_id, Some(card_id)) = (session.deck, session.current()) else {
            return;
        };
        let Some(card) = self.db.deck_by_id(deck_id).and_then(|d| d.card(card_id)) else {
            return;
        };
        self.editor = Some(Editor::from_card(deck_id, card));
        self.mode = Mode::Normal;
    }

    /// Write the editor back into the collection. Returns false on a
    /// validation error so the editor can stay open.
    fn commit_editor(&mut self) -> bool {
        let Some(editor) = self.editor.as_ref() else {
            return true;
        };
        let values: Vec<String> = editor
            .fields
            .iter()
            .map(|f| unescape(f.value().trim()))
            .collect();

        if values[0].is_empty() {
            self.error("a card needs a term");
            return false;
        }
        if values[3].is_empty() {
            self.error("a card needs a definition");
            return false;
        }

        let deck_id = editor.deck();
        let target = editor.target;
        let today = self.today;

        // Past the validation above, so a card the editor refuses to save
        // leaves nothing behind for `u`.
        self.checkpoint(match target {
            EditTarget::New { .. } => format!("adding {}", values[0]),
            EditTarget::Existing { .. } => format!("editing {}", values[0]),
        });

        match target {
            EditTarget::New { .. } => {
                let id = self.db.next_card_id();
                let Some(deck) = self.db.deck_by_id_mut(deck_id) else {
                    self.undo.pop();
                    self.error("deck vanished");
                    return false;
                };
                let mut card = Card::new(id, values[0].clone(), values[3].clone(), today);
                card.pron = values[1].clone();
                card.pos = values[2].clone();
                card.example = values[4].clone();
                card.notes = values[5].clone();
                deck.cards.push(card);
                self.info(format!("added {}", values[0]));
            }
            EditTarget::Existing { card, .. } => {
                let Some(existing) = self
                    .db
                    .deck_by_id_mut(deck_id)
                    .and_then(|d| d.card_mut(card))
                else {
                    self.undo.pop();
                    self.error("card vanished");
                    return false;
                };
                existing.term = values[0].clone();
                existing.pron = values[1].clone();
                existing.pos = values[2].clone();
                existing.definition = values[3].clone();
                existing.example = values[4].clone();
                existing.notes = values[5].clone();
                self.info(format!("saved {}", values[0]));
            }
        }

        self.editor = None;
        self.mode = Mode::Normal;
        self.dirty = true;
        self.refresh_rows();
        true
    }

    fn close_editor(&mut self) {
        self.editor = None;
        self.mode = Mode::Normal;
        self.info("cancelled");
    }

    fn delete_selected_card(&mut self) {
        let Some((deck_id, card_index)) = self.selected_card() else {
            self.error("no card selected");
            return;
        };
        let Some(deck) = self.db.deck_by_id_mut(deck_id) else {
            self.error("no card selected");
            return;
        };
        if card_index >= deck.cards.len() {
            self.error("no card selected");
            return;
        }
        let term = deck.cards[card_index].term.clone();
        self.checkpoint(format!("deleting {term}"));
        if let Some(deck) = self.db.deck_by_id_mut(deck_id) {
            deck.cards.remove(card_index);
        }
        self.dirty = true;
        self.refresh_rows();
        self.info(format!("deleted {term} — u to undo"));
    }

    // -- undo --------------------------------------------------------------

    /// Record the collection as it stands, ahead of the change `label`
    /// describes. Every path that sets `dirty` calls this first, and calls it
    /// after its own validation, so a refused change leaves no empty step for
    /// `u` to walk through.
    fn checkpoint(&mut self, label: impl Into<String>) {
        // A new change makes the redo stack unreachable, as in Vim.
        self.redo.clear();
        if self.undo.len() == UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.undo.push(self.step(label.into()));
    }

    /// The here and now, ready to be pushed onto either stack.
    fn step(&self, label: String) -> Step {
        Step {
            db: self.db.clone(),
            label,
            screen: self.screen,
            deck: self.browse_deck,
            row: self.browse_sel,
        }
    }

    /// `u` — take back the last `count` changes.
    fn undo(&mut self, count: usize) {
        self.walk(count, true);
    }

    /// `Ctrl-r` — put back the last `count` changes that `u` took away.
    fn redo(&mut self, count: usize) {
        self.walk(count, false);
    }

    fn walk(&mut self, count: usize, back: bool) {
        let mut last = None;
        let mut moved = 0;
        for _ in 0..count.max(1) {
            let from = if back { &mut self.undo } else { &mut self.redo };
            let Some(step) = from.pop() else { break };
            let here = self.step(step.label.clone());
            if back {
                self.redo.push(here);
            } else {
                self.undo.push(here);
            }
            last = Some(step.label.clone());
            self.restore_step(step);
            moved += 1;
        }

        let Some(label) = last else {
            // Vim's own wording, because this is Vim's own situation.
            self.error(if back {
                "already at oldest change"
            } else {
                "already at newest change"
            });
            return;
        };

        self.dirty = true;
        let verb = if back { "undid" } else { "redid" };
        if moved > 1 {
            self.info(format!("{verb} {moved} changes, back to {label}"));
        } else {
            self.info(format!("{verb} {label}"));
        }
    }

    /// Put a recorded state back on screen, cursor included.
    fn restore_step(&mut self, step: Step) {
        self.db = step.db;
        self.browse_deck = step.deck;
        self.browse_sel = step.row;

        // The deck the step was taken in may not exist in the state being
        // restored — undoing `:deck foo` is exactly that case — and a browser
        // pointed at a deck that is not there shows "no deck" and traps you.
        let deck_gone = self.db.deck_by_id(self.browse_deck).is_none();
        self.screen = match step.screen {
            Screen::Browse if deck_gone => Screen::Decks,
            // A session's queue holds card ids that a restored collection need
            // not contain, so undo never drops you back into one.
            Screen::Review => Screen::Decks,
            screen => screen,
        };
        if self.screen != Screen::Review {
            self.session = None;
            self.last_answer = None;
        }
        self.peek = false;
        self.refresh_rows();
        self.clamp_decks_sel();
    }

    fn clamp_decks_sel(&mut self) {
        if self.decks_sel >= self.db.decks.len() {
            self.decks_sel = self.db.decks.len().saturating_sub(1);
        }
    }

    /// The card the current screen is pointing at, as `(deck id, card id)`.
    fn current_card(&self) -> Option<(u64, u64)> {
        match self.screen {
            Screen::Review => self
                .session
                .as_ref()
                .and_then(|s| s.current().map(|id| (s.deck, id))),
            Screen::Browse => self.selected_card().and_then(|(deck, index)| {
                let card = self.db.deck_by_id(deck)?.cards.get(index)?;
                Some((deck, card.id))
            }),
            _ => None,
        }
    }

    /// `m` — step this word to the next status; `M` is the drill, so the
    /// reverse step lives on `Ctrl-m`.
    fn cycle_status(&mut self, forward: bool) {
        let Some((deck_id, card_id)) = self.current_card() else {
            self.error("no card selected");
            return;
        };
        let Some(term) = self
            .db
            .deck_by_id(deck_id)
            .and_then(|d| d.card(card_id))
            .map(|c| c.term.clone())
        else {
            return;
        };
        self.checkpoint(format!("the status of {term}"));
        let Some(card) = self
            .db
            .deck_by_id_mut(deck_id)
            .and_then(|d| d.card_mut(card_id))
        else {
            return;
        };
        card.status = if forward {
            card.status.next()
        } else {
            card.status.prev()
        };
        let (term, status) = (card.term.clone(), card.status);
        self.dirty = true;
        self.refresh_rows();
        self.info(match status {
            Study::None => format!("{term}: status cleared"),
            other => format!("{term}: {} — m again to change", other.label()),
        });
    }

    /// `:status <name>` — set this word's status outright.
    fn set_status(&mut self, status: Study) {
        let Some((deck_id, card_id)) = self.current_card() else {
            self.error("no card selected");
            return;
        };
        let Some(term) = self
            .db
            .deck_by_id(deck_id)
            .and_then(|d| d.card(card_id))
            .map(|c| c.term.clone())
        else {
            return;
        };
        self.checkpoint(format!("the status of {term}"));
        let Some(card) = self
            .db
            .deck_by_id_mut(deck_id)
            .and_then(|d| d.card_mut(card_id))
        else {
            return;
        };
        card.status = status;
        let term = card.term.clone();
        self.dirty = true;
        self.refresh_rows();
        self.info(match status {
            Study::None => format!("{term}: status cleared"),
            other => format!("{term}: {}", other.label()),
        });
    }

    fn toggle_suspend(&mut self) {
        let Some((deck_id, card_index)) = self.selected_card() else {
            return;
        };
        let Some(card) = self
            .db
            .deck_by_id(deck_id)
            .and_then(|d| d.cards.get(card_index))
        else {
            return;
        };
        self.checkpoint(if card.suspended {
            format!("unsuspending {}", card.term)
        } else {
            format!("suspending {}", card.term)
        });
        let Some(card) = self
            .db
            .deck_by_id_mut(deck_id)
            .and_then(|d| d.cards.get_mut(card_index))
        else {
            return;
        };
        card.suspended = !card.suspended;
        let (term, suspended) = (card.term.clone(), card.suspended);
        self.dirty = true;
        self.info(if suspended {
            format!("suspended {term}")
        } else {
            format!("unsuspended {term}")
        });
    }

    // -- navigation --------------------------------------------------------

    fn list_len(&self) -> usize {
        match self.screen {
            Screen::Decks => self.db.decks.len(),
            Screen::Browse => self.rows.len(),
            _ => 0,
        }
    }

    fn cursor(&mut self) -> &mut usize {
        match self.screen {
            Screen::Browse => &mut self.browse_sel,
            _ => &mut self.decks_sel,
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        self.peek = false;
        let cursor = self.cursor();
        let next = (*cursor as isize + delta).clamp(0, len as isize - 1);
        *cursor = next as usize;
    }

    fn goto(&mut self, index: usize) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        self.peek = false;
        *self.cursor() = index.min(len - 1);
    }

    /// `n` / `N` — jump to the next row matching the filter.
    fn jump_match(&mut self, forward: bool) {
        if self.filter.is_empty() || self.rows.is_empty() {
            return;
        }
        self.peek = false;
        let len = self.rows.len();
        self.browse_sel = if forward {
            (self.browse_sel + 1) % len
        } else {
            (self.browse_sel + len - 1) % len
        };
    }

    /// `z` — cover the meanings, or uncover them if they are already covered.
    /// The whole list at once: it is the list you are testing yourself against.
    fn toggle_hide_meanings(&mut self) {
        self.hide_meanings = !self.hide_meanings;
        self.peek = false;
        if self.hide_meanings {
            self.info("meanings hidden — space to check one, z to show them all");
        } else {
            self.info("meanings shown");
        }
    }

    /// `space` — uncover just the word under the cursor, so checking one
    /// answer does not mean giving away the rest of the list.
    fn toggle_peek(&mut self) {
        if !self.hide_meanings {
            return;
        }
        self.peek = !self.peek;
    }

    /// Whether the selected card's meaning is on screen: covered by `z`, minus
    /// the one word `space` has uncovered.
    pub fn meaning_hidden(&self, row: usize) -> bool {
        self.hide_meanings && !(self.peek && row == self.browse_sel)
    }

    fn enter_browse(&mut self, deck_id: u64) {
        self.browse_deck = deck_id;
        self.browse_sel = 0;
        self.screen = Screen::Browse;
        self.refresh_rows();
    }

    fn back(&mut self) {
        match self.screen {
            Screen::Help | Screen::Stats => {
                self.screen = self.help_from;
            }
            Screen::Review => {
                let answered = self.session.as_ref().map(|s| s.answered).unwrap_or(0);
                self.session = None;
                self.screen = Screen::Decks;
                self.info(format!("session paused after {answered} answers"));
                self.autosave();
            }
            Screen::Browse => {
                self.screen = Screen::Decks;
                self.filter.clear();
            }
            Screen::Decks => self.request_quit(),
        }
    }

    fn request_quit(&mut self) {
        if self.dirty {
            match self.save() {
                Ok(_) => self.quit = true,
                Err(err) => {
                    self.error(format!("save failed: {err:#} — :q! to quit anyway"));
                }
            }
        } else {
            self.quit = true;
        }
    }

    /// Quit without saving, confirming first if there is anything to lose.
    fn request_discard_quit(&mut self) {
        if self.dirty {
            self.confirm = Some(Confirm {
                prompt: "discard unsaved changes? [y/N]".into(),
                action: ConfirmAction::DiscardAndQuit,
            });
        } else {
            self.quit = true;
        }
    }

    fn open_stats(&mut self) {
        if self.screen != Screen::Stats {
            self.help_from = self.screen;
        }
        self.screen = Screen::Stats;
        self.help_scroll = 0;
    }

    // -- key handling ------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        if self.confirm.is_some() {
            self.key_confirm(key);
            return Ok(());
        }
        match self.mode {
            Mode::Command | Mode::Search => self.key_line(key),
            _ if self.editor.is_some() => self.key_editor(key),
            _ => self.key_screen(key),
        }
        Ok(())
    }

    fn key_confirm(&mut self, key: KeyEvent) {
        let answer = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => true,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => false,
            _ => return,
        };
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        if !answer {
            self.info("cancelled");
            return;
        }
        match confirm.action {
            ConfirmAction::DeleteDeck(deck_id) => {
                if let Some(index) = self.db.index_of(deck_id) {
                    self.checkpoint(format!("deleting deck {}", self.db.decks[index].name));
                    let deck = self.db.decks.remove(index);
                    self.decks_sel = self.decks_sel.min(self.db.decks.len().saturating_sub(1));
                    // Anything still pointing at this deck now points nowhere.
                    if self.browse_deck == deck_id {
                        self.browse_deck = 0;
                    }
                    if self.session.as_ref().is_some_and(|s| s.deck == deck_id) {
                        self.session = None;
                        if self.screen == Screen::Review {
                            self.screen = Screen::Decks;
                        }
                    }
                    self.last_answer = None;
                    self.dirty = true;
                    self.refresh_rows();
                    self.info(format!("deleted deck {}", deck.name));
                }
            }
            ConfirmAction::ForgetDeck(deck_id) => {
                let today = self.today;
                if let Some(name) = self.db.deck_by_id(deck_id).map(|d| d.name.clone()) {
                    self.checkpoint(format!("resetting {name}"));
                }
                if let Some(deck) = self.db.deck_by_id_mut(deck_id) {
                    for card in &mut deck.cards {
                        card.forget(today);
                    }
                    deck.intro_date = None;
                    deck.intro_count = 0;
                    let count = deck.cards.len();
                    self.dirty = true;
                    self.info(format!("reset scheduling for {count} cards"));
                }
            }
            ConfirmAction::DiscardAndQuit => {
                self.dirty = false;
                self.quit = true;
            }
            ConfirmAction::Restore(backup) => self.restore(&backup),
        }
    }

    /// `:restore` — put a kept copy back, or say which copies there are.
    fn begin_restore(&mut self, rest: &str) {
        // Newest first: the one worth restoring is almost always the last
        // conflict, and it should be `1`.
        let mut copies = store::conflict_copies(&self.path);
        copies.reverse();

        // The collection's own directory, which `gram path` prints — naming it
        // here only costs the room the message needs.
        let none_kept = "no kept copies beside the collection — nothing to restore";

        if rest.is_empty() {
            if copies.is_empty() {
                self.error(none_kept);
                return;
            }
            const SHOWN: usize = 5;
            let listed = copies
                .iter()
                .take(SHOWN)
                .enumerate()
                .map(|(i, path)| format!("{}: {}", i + 1, store::backup_label(path)))
                .collect::<Vec<_>>()
                .join("  ");
            // A status line holds only so much. Say what is not being shown
            // rather than looking like a complete list — the hidden ones are
            // still reachable by number.
            let rest = match copies.len().saturating_sub(SHOWN) {
                0 => String::new(),
                n => format!(" (+{n} older, still :restore-able by number)"),
            };
            self.info(format!("{listed}{rest} — :restore <n> to put one back"));
            return;
        }

        let chosen = match rest.parse::<usize>() {
            Ok(n) if n >= 1 && n <= copies.len() => copies[n - 1].clone(),
            // `:restore 3` with two copies is a different mistake from
            // `:restore 3` with none, and deserves a different answer.
            Ok(_) if copies.is_empty() => {
                self.error(none_kept);
                return;
            }
            Ok(n) => {
                self.error(format!(
                    "no kept copy {n} — there {} — :restore lists them",
                    match copies.len() {
                        1 => "is 1".to_string(),
                        many => format!("are {many}"),
                    }
                ));
                return;
            }
            // Not a number, so it is a path: any file gram can read will do,
            // which is how an external backup gets loaded.
            Err(_) => match store::expand_tilde(rest) {
                Ok(path) => path,
                Err(err) => {
                    self.error(format!("{err}"));
                    return;
                }
            },
        };
        if !chosen.exists() {
            self.error(format!("{} does not exist", chosen.display()));
            return;
        }

        let name = store::backup_label(&chosen);
        let prompt = if self.dirty {
            format!("restore {name}, losing this session's unsaved answers? [y/N]")
        } else {
            format!("replace the collection with {name}? [y/N]")
        };
        self.confirm = Some(Confirm {
            prompt,
            action: ConfirmAction::Restore(chosen),
        });
    }

    /// Write the backup over the collection and start again from it. Nothing
    /// held about the old collection survives: ids in a session or an undo
    /// stack mean nothing against a different set of cards.
    fn restore(&mut self, backup: &Path) {
        let displaced = match store::restore_from(backup, &self.path) {
            Ok(displaced) => displaced,
            Err(err) => {
                self.error(format!("restore failed: {err:#}"));
                return;
            }
        };
        let db = match store::load_from(&self.path) {
            Ok(db) => db,
            Err(err) => {
                self.error(format!("restored, but reloading failed: {err:#}"));
                return;
            }
        };

        self.db = db;
        self.session = None;
        self.editor = None;
        self.last_answer = None;
        self.undo.clear();
        self.redo.clear();
        self.browse_deck = 0;
        self.browse_sel = 0;
        self.filter.clear();
        self.status_filter = StatusFilter::default();
        self.decks_sel = 0;
        self.card_scroll = 0;
        self.screen = Screen::Decks;
        self.written = store::modified(&self.path);
        self.dirty = false;
        self.refresh_rows();

        let cards: usize = self.db.decks.iter().map(|deck| deck.cards.len()).sum();
        let name = store::backup_label(backup);
        match displaced {
            // Stamps, not paths: a full path is the first thing the status
            // line truncates away, and it is exactly the part you would need.
            Some(aside) => self.info(format!(
                "restored {name} — {cards} cards; old copy kept as {}",
                store::backup_label(&aside)
            )),
            None => self.info(format!("restored {name} — {cards} cards")),
        }
    }

    /// `:` and `/` share one editable line.
    fn key_line(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Tab completes `:` commands; anything else typed ends the completion
        // the last Tab started.
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            if self.mode == Mode::Command {
                self.complete_line(key.code == KeyCode::Tab);
            }
            return;
        }
        self.completion = None;

        match key.code {
            KeyCode::Esc => {
                self.line.clear();
                // The editor is tracked separately, so leaving the command
                // line always just means "back to normal mode".
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                let text = self.line.take();
                let searching = self.mode == Mode::Search;
                self.mode = Mode::Normal;
                if searching {
                    self.filter = text;
                    self.browse_sel = 0;
                    self.refresh_rows();
                    let hits = self.rows.len();
                    if hits == 0 {
                        self.error(format!("no cards match {:?}", self.filter));
                    } else {
                        self.info(format!("{hits} matches"));
                    }
                } else {
                    self.run_command(&text);
                }
            }
            KeyCode::Backspace => {
                if self.line.is_empty() {
                    self.mode = Mode::Normal;
                } else {
                    self.line.delete_back();
                }
            }
            KeyCode::Left => self.line.left(),
            KeyCode::Right => self.line.right(true),
            KeyCode::Home => self.line.home(),
            KeyCode::End => self.line.end(true),
            KeyCode::Char('w') if ctrl => self.line.delete_word_back(),
            KeyCode::Char('u') if ctrl => self.line.delete_to_start(),
            KeyCode::Char(ch) if !ctrl => self.line.insert(ch),
            _ => {}
        }
    }

    /// Tab on the command line. The first press fills in as far as the
    /// candidates agree; once there is nothing left to fill, further presses
    /// cycle through them, and `BackTab` cycles the other way.
    fn complete_line(&mut self, forward: bool) {
        if let Some(mut state) = self.completion.take() {
            let count = state.candidates.len();
            let next = match state.index {
                Some(index) if forward => (index + 1) % count,
                Some(index) => (index + count - 1) % count,
                None if forward => 0,
                None => count - 1,
            };
            state.index = Some(next);
            self.line.replace_from(state.at, &state.candidates[next]);
            self.completion = Some(state);
            return;
        }

        let (at, candidates) = complete::candidates(self.line.value());
        let [only] = &candidates[..] else {
            if candidates.is_empty() {
                return;
            }
            // Fill in the shared prefix first: `:st` becomes `:stat` with
            // stats and status still on the table.
            let common = complete::common_prefix(&candidates);
            let typed = self.line.len() - at;
            if common.chars().count() > typed {
                self.line.replace_from(at, &common);
                self.completion = Some(Completion {
                    at,
                    candidates,
                    index: None,
                });
            } else {
                // Nothing left to fill, so start cycling right away.
                self.completion = Some(Completion {
                    at,
                    candidates,
                    index: None,
                });
                self.complete_line(forward);
            }
            return;
        };

        // One candidate: take it, and space along to the argument. A trailing
        // `/` or `=` means the word itself is unfinished — `decks/` has a file
        // to come, `grading=` a value — so the space waits.
        let unfinished = only.ends_with('/') || only.ends_with('=');
        let single = if unfinished {
            only.clone()
        } else {
            format!("{only} ")
        };
        self.line.replace_from(at, &single);
    }

    fn key_editor(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let insert = self.mode == Mode::Insert;
        let fields = FIELDS.len();

        // Keys that behave the same in both modes.
        match key.code {
            KeyCode::Tab => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.field = (editor.field + 1) % fields;
                }
                return;
            }
            KeyCode::BackTab => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.field = (editor.field + fields - 1) % fields;
                }
                return;
            }
            KeyCode::Char('s') if ctrl => {
                self.commit_editor();
                self.autosave();
                return;
            }
            _ => {}
        }

        if insert {
            match key.code {
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    if let Some(editor) = self.editor.as_mut() {
                        editor.current_mut().clamp_normal();
                    }
                }
                KeyCode::Enter => {
                    if let Some(editor) = self.editor.as_mut() {
                        if editor.field + 1 < fields {
                            editor.field += 1;
                        } else {
                            self.commit_editor();
                        }
                    }
                }
                KeyCode::Backspace => self.field_mut(|f| f.delete_back()),
                KeyCode::Delete => self.field_mut(|f| f.delete_under()),
                KeyCode::Left => self.field_mut(|f| f.left()),
                KeyCode::Right => self.field_mut(|f| f.right(true)),
                KeyCode::Home => self.field_mut(|f| f.home()),
                KeyCode::End => self.field_mut(|f| f.end(true)),
                KeyCode::Char('w') if ctrl => self.field_mut(|f| f.delete_word_back()),
                KeyCode::Char('u') if ctrl => self.field_mut(|f| f.delete_to_start()),
                KeyCode::Char('k') if ctrl => self.field_mut(|f| f.delete_to_end()),
                KeyCode::Char(ch) if !ctrl => self.field_mut(|f| f.insert(ch)),
                _ => {}
            }
            return;
        }

        // Normal mode inside the editor.
        let pending = self.pending.take();
        match (pending, key.code) {
            (Some('d'), KeyCode::Char('d')) => self.field_mut(|f| {
                f.clear();
            }),
            (Some('g'), KeyCode::Char('g')) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.field = 0;
                }
            }
            (_, KeyCode::Char('d')) => self.pending = Some('d'),
            (_, KeyCode::Char('g')) => self.pending = Some('g'),
            (_, KeyCode::Char('G')) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.field = fields - 1;
                }
            }
            (_, KeyCode::Char('i')) => self.mode = Mode::Insert,
            (_, KeyCode::Char('a')) => {
                self.field_mut(|f| f.right(true));
                self.mode = Mode::Insert;
            }
            (_, KeyCode::Char('I')) => {
                self.field_mut(|f| f.home());
                self.mode = Mode::Insert;
            }
            (_, KeyCode::Char('A')) => {
                self.field_mut(|f| f.end(true));
                self.mode = Mode::Insert;
            }
            (_, KeyCode::Char('C')) => {
                self.field_mut(|f| f.delete_to_end());
                self.mode = Mode::Insert;
            }
            (_, KeyCode::Char('D')) => self.field_mut(|f| f.delete_to_end()),
            (_, KeyCode::Char('x')) => self.field_mut(|f| f.delete_under()),
            (_, KeyCode::Char('h')) | (_, KeyCode::Left) => self.field_mut(|f| f.left()),
            (_, KeyCode::Char('l')) | (_, KeyCode::Right) => self.field_mut(|f| f.right(false)),
            (_, KeyCode::Char('0')) | (_, KeyCode::Home) => self.field_mut(|f| f.home()),
            (_, KeyCode::Char('$')) | (_, KeyCode::End) => self.field_mut(|f| f.end(false)),
            (_, KeyCode::Char('w')) => self.field_mut(|f| f.word_forward()),
            (_, KeyCode::Char('b')) => self.field_mut(|f| f.word_back()),
            (_, KeyCode::Char('j')) | (_, KeyCode::Down) | (_, KeyCode::Enter) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.field = (editor.field + 1) % fields;
                }
            }
            (_, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.field = (editor.field + fields - 1) % fields;
                }
            }
            (_, KeyCode::Char('Z')) => {
                if pending == Some('Z') {
                    self.commit_editor();
                    self.autosave();
                } else {
                    self.pending = Some('Z');
                }
            }
            (_, KeyCode::Char(':')) => {
                self.mode = Mode::Command;
                self.line.clear();
            }
            (_, KeyCode::Char('q')) | (_, KeyCode::Esc) => self.close_editor(),
            _ => {}
        }
    }

    fn field_mut(&mut self, f: impl FnOnce(&mut TextInput)) {
        if let Some(editor) = self.editor.as_mut() {
            f(editor.current_mut());
        }
    }

    fn key_screen(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Numeric prefix, Vim style. `0` is a motion when no count is pending.
        if let KeyCode::Char(ch @ '0'..='9') = key.code {
            let counting = ch != '0' || !self.count.is_empty();
            let grading = self.screen == Screen::Review;
            if counting && !grading {
                self.count.push(ch);
                return;
            }
        }

        let pending = self.pending.take();
        if let Some('g') = pending {
            match key.code {
                KeyCode::Char('g') => {
                    let index = self.take_count().saturating_sub(1);
                    self.goto(index);
                }
                _ => self.error("unknown command: g"),
            }
            return;
        }
        if let Some('d') = pending {
            self.count.clear();
            if key.code == KeyCode::Char('d') {
                self.delete_row();
            }
            return;
        }
        if let Some('Z') = pending {
            self.count.clear();
            match key.code {
                KeyCode::Char('Z') => self.request_quit(),
                KeyCode::Char('Q') => {
                    self.dirty = false;
                    self.quit = true;
                }
                _ => {}
            }
            return;
        }

        // Screen-specific keys get first refusal, then the shared bindings.
        if self.screen == Screen::Review && self.key_review(key) {
            return;
        }

        // An operator holds any pending count for its second keystroke, so
        // `5gg` and `5G` work. Consuming the count before this point would
        // throw it away on the first press.
        if !ctrl {
            match key.code {
                KeyCode::Char('g') => {
                    self.pending = Some('g');
                    return;
                }
                KeyCode::Char('d') => {
                    self.pending = Some('d');
                    return;
                }
                KeyCode::Char('Z') => {
                    self.pending = Some('Z');
                    return;
                }
                _ => {}
            }
        }

        // Every other key consumes the count, which also clears it.
        let explicit = self.take_count_opt();
        let count = explicit.unwrap_or(1) as isize;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.back(),
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.line.clear();
            }
            KeyCode::Char('/') if self.screen == Screen::Browse => {
                self.mode = Mode::Search;
                self.line.clear();
            }
            KeyCode::Char('?') => {
                if self.screen != Screen::Help {
                    self.help_from = self.screen;
                }
                self.screen = Screen::Help;
                self.help_scroll = 0;
            }
            KeyCode::Char('n') if self.screen == Screen::Browse => self.jump_match(true),
            KeyCode::Char('N') if self.screen == Screen::Browse => self.jump_match(false),

            KeyCode::Char('j') | KeyCode::Down => self.scroll(count),
            KeyCode::Char('k') | KeyCode::Up => self.scroll(-count),
            KeyCode::Char('d') if ctrl => self.scroll(10),
            KeyCode::Char('u') if ctrl => self.scroll(-10),
            KeyCode::Char('f') if ctrl => self.scroll(20),
            KeyCode::Char('b') if ctrl => self.scroll(-20),
            KeyCode::PageDown => self.scroll(20),
            KeyCode::PageUp => self.scroll(-20),
            KeyCode::Char('G') => match explicit {
                _ if self.screen == Screen::Help => self.help_scroll = u16::MAX / 2,
                Some(line) => self.goto(line.saturating_sub(1)),
                None => self.goto(usize::MAX),
            },

            KeyCode::Char('s') => {
                if let Some(deck) = self.active_deck() {
                    self.start_session(deck, Scope::Due);
                }
            }
            KeyCode::Char('M') => {
                if let Some(deck) = self.active_deck() {
                    self.start_session(deck, Scope::Flagged(None));
                }
            }
            KeyCode::Char('m') if self.screen == Screen::Browse => self.cycle_status(!ctrl),
            KeyCode::Char('z') if self.screen == Screen::Browse => self.toggle_hide_meanings(),
            KeyCode::Char(' ') if self.screen == Screen::Browse => self.toggle_peek(),
            KeyCode::Char('S') => self.open_stats(),
            KeyCode::Char('a') | KeyCode::Char('o') => self.open_new_card(),
            // `u` and Ctrl-r walk the change history, as in Vim. Inside a
            // review session `u` means something finer — undo my last answer —
            // and `key_review` has already had its refusal above.
            KeyCode::Char('u') => self.undo(count.max(1) as usize),
            KeyCode::Char('r') if ctrl => self.redo(count.max(1) as usize),
            KeyCode::Char('x') if self.screen == Screen::Browse => self.toggle_suspend(),

            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => match self.screen {
                Screen::Decks => {
                    if let Some(deck) = self.active_deck() {
                        self.enter_browse(deck);
                    }
                }
                Screen::Browse => self.open_edit_card(),
                _ => {}
            },
            KeyCode::Char('h') | KeyCode::Left => {
                if self.screen != Screen::Decks {
                    self.back();
                }
            }
            KeyCode::Char('i') if self.screen == Screen::Browse => self.open_edit_card(),
            _ => {}
        }
    }

    fn scroll(&mut self, delta: isize) {
        // Clamp before narrowing: a big count such as `99999j` would
        // otherwise wrap round to a negative i16 and scroll backwards.
        let narrow = |delta: isize| delta.clamp(i16::MIN as isize, i16::MAX as isize) as i16;
        match self.screen {
            Screen::Help | Screen::Stats => {
                self.help_scroll = self.help_scroll.saturating_add_signed(narrow(delta));
            }
            // A long answer scrolls inside the card. The render clamps it to
            // whatever actually overflows.
            Screen::Review => {
                self.card_scroll = self.card_scroll.saturating_add_signed(narrow(delta));
            }
            _ => self.move_cursor(delta),
        }
    }

    fn delete_row(&mut self) {
        match self.screen {
            Screen::Browse => self.delete_selected_card(),
            Screen::Decks => {
                let Some(deck) = self.active_deck().and_then(|id| self.db.deck_by_id(id)) else {
                    return;
                };
                self.confirm = Some(Confirm {
                    prompt: format!(
                        "delete deck \"{}\" and its {} cards? [y/N]",
                        deck.name,
                        deck.cards.len()
                    ),
                    action: ConfirmAction::DeleteDeck(deck.id),
                });
            }
            _ => {}
        }
    }

    /// Turn the card over, from the top of the answer.
    fn reveal(&mut self) {
        if let Some(session) = self.session.as_mut() {
            session.revealed = true;
        }
        self.card_scroll = 0;
    }

    /// Review-screen keys. Returns true when the key was consumed.
    fn key_review(&mut self, key: KeyEvent) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        let revealed = session.revealed;

        match key.code {
            KeyCode::Char(' ') | KeyCode::Enter if !revealed => {
                self.reveal();
                true
            }
            KeyCode::Char(' ') | KeyCode::Enter if revealed => {
                self.answer(Grade::Good);
                true
            }
            KeyCode::Char(ch @ '1'..='4') if revealed => {
                if let Some(grade) = Grade::from_index(ch as u8 - b'1') {
                    self.answer(grade);
                }
                true
            }
            KeyCode::Char(ch @ '1'..='4') if !revealed => {
                // Grading blind is almost always a mistake; reveal instead.
                let _ = ch;
                self.reveal();
                true
            }
            // "I know this" / "I don't know" — the binary answer buttons.
            KeyCode::Char('y') if revealed => {
                self.answer(Grade::Good);
                true
            }
            KeyCode::Char('n') if revealed => {
                self.answer(Grade::Again);
                true
            }
            KeyCode::Char('y') | KeyCode::Char('n') if !revealed => {
                self.reveal();
                true
            }
            KeyCode::Char('m') => {
                let back = key.modifiers.contains(KeyModifiers::CONTROL);
                self.cycle_status(!back);
                true
            }
            KeyCode::Char('u') => {
                self.undo_answer();
                true
            }
            KeyCode::Char('e') => {
                self.edit_review_card();
                true
            }
            KeyCode::Char('x') => {
                let Some(session) = self.session.as_mut() else {
                    return true;
                };
                let Some(card_id) = session.queue.pop_front() else {
                    return true;
                };
                session.revealed = false;
                // Out of the queue for good, so it counts as dealt with —
                // otherwise the bar could never reach the end.
                session.retrying.remove(&card_id);
                session.completed += 1;
                let deck_id = session.deck;
                self.card_scroll = 0;
                if let Some(card) = self
                    .db
                    .deck_by_id_mut(deck_id)
                    .and_then(|d| d.card_mut(card_id))
                {
                    card.suspended = true;
                    let term = card.term.clone();
                    self.dirty = true;
                    self.info(format!("suspended {term}"));
                }
                if self.session.as_ref().is_some_and(|s| s.queue.is_empty()) {
                    self.finish_session();
                }
                true
            }
            KeyCode::Char('S') if !revealed => {
                // Bury: push to the back of today's queue.
                if let Some(session) = self.session.as_mut()
                    && let Some(card_id) = session.queue.pop_front()
                {
                    session.queue.push_back(card_id);
                }
                self.card_scroll = 0;
                self.info("buried until later in the session");
                true
            }
            _ => false,
        }
    }

    /// Consume the pending count, if the user typed one.
    fn take_count_opt(&mut self) -> Option<usize> {
        if self.count.is_empty() {
            return None;
        }
        let value = self.count.parse().ok();
        self.count.clear();
        value
    }

    fn take_count(&mut self) -> usize {
        self.take_count_opt().unwrap_or(1)
    }

    // -- ex commands -------------------------------------------------------

    fn run_command(&mut self, raw: &str) {
        let text = raw.trim();
        if text.is_empty() {
            return;
        }
        // `:42` jumps to a row.
        if let Ok(line) = text.parse::<usize>() {
            self.goto(line.saturating_sub(1));
            return;
        }

        let (name, rest) = match text.split_once(char::is_whitespace) {
            Some((name, rest)) => (name, rest.trim()),
            None => (text, ""),
        };

        match name {
            "w" | "write" => {
                if self.editor.is_some() && !self.commit_editor() {
                    return;
                }
                match self.save() {
                    Ok(conflict) => {
                        let path = self.path.display().to_string();
                        match conflict {
                            None => self.info(format!("written to {path}")),
                            // Someone else had written to it; say where their
                            // version went rather than just claiming success.
                            Some(aside) => self.error(format!(
                                "written to {path} — the version already there kept as {}",
                                store::backup_label(&aside)
                            )),
                        }
                    }
                    Err(err) => self.error(format!("save failed: {err:#}")),
                }
            }
            "restore" => self.begin_restore(rest),
            "q" | "quit" => {
                if self.editor.is_some() {
                    self.close_editor();
                } else {
                    self.back();
                }
            }
            "q!" | "quit!" => self.request_discard_quit(),
            "wq" | "x" | "wqa" | "xa" => {
                if self.editor.is_some() && !self.commit_editor() {
                    return;
                }
                self.request_quit();
            }
            // `:qa` saves like `ZZ`; only the banged form throws work away,
            // and even then it asks first.
            "qa" | "quitall" => {
                if self.editor.is_some() {
                    self.close_editor();
                }
                self.request_quit();
            }
            "qa!" | "quitall!" => {
                if self.editor.is_some() {
                    self.close_editor();
                }
                self.request_discard_quit();
            }
            "deck" | "mkdeck" => {
                if rest.is_empty() {
                    self.error("usage: :deck <name>");
                    return;
                }
                if self.db.deck_index(rest).is_some() {
                    self.error(format!("deck {rest:?} already exists"));
                    return;
                }
                self.checkpoint(format!("creating deck {rest}"));
                let index = self.db.add_deck(rest.to_string());
                self.decks_sel = index;
                self.dirty = true;
                self.info(format!("created deck {rest}"));
            }
            "rename" => {
                let Some(deck_id) = self.active_deck() else {
                    self.error("no deck selected");
                    return;
                };
                if rest.is_empty() {
                    self.error("usage: :rename <name>");
                    return;
                }
                let was = self
                    .db
                    .deck_by_id(deck_id)
                    .map_or_else(String::new, |d| d.name.clone());
                self.checkpoint(format!("renaming {was}"));
                if let Some(deck) = self.db.deck_by_id_mut(deck_id) {
                    deck.name = rest.to_string();
                }
                self.dirty = true;
                self.info(format!("renamed to {rest}"));
            }
            "import" => self.cmd_import(rest),
            "export" => self.cmd_export(rest),
            "study" | "s" => {
                let scope = match rest {
                    "" | "due" => Scope::Due,
                    "flagged" | "all" => Scope::Flagged(None),
                    other => match Study::parse(other) {
                        Some(Study::None) => Scope::Flagged(None),
                        Some(status) => Scope::Flagged(Some(status)),
                        None => {
                            self.error(format!(
                                "unknown study scope: {other} (try: due, flagged, review, marked, test)"
                            ));
                            return;
                        }
                    },
                };
                if let Some(deck_id) = self.active_deck() {
                    self.start_session(deck_id, scope);
                }
            }
            // `:marked` toggles the flagged-only view; `:marked test` narrows
            // it to one status.
            "marked" | "mark" | "flagged" => {
                self.status_filter = match (rest, self.status_filter) {
                    ("", StatusFilter::Off) => StatusFilter::Any,
                    ("", _) => StatusFilter::Off,
                    (name, _) => match Study::parse(name) {
                        Some(Study::None) => StatusFilter::Off,
                        Some(status) => StatusFilter::Only(status),
                        None => {
                            self.error(format!(
                                "unknown status: {name} (try: review, marked, test, none)"
                            ));
                            return;
                        }
                    },
                };
                if let Some(deck_id) = self.active_deck()
                    && self.screen != Screen::Browse
                {
                    self.enter_browse(deck_id);
                }
                self.browse_sel = 0;
                self.refresh_rows();
                let shown = self.rows.len();
                if self.status_filter.is_on() {
                    self.info(format!(
                        "{shown} {} words — :noh to show all",
                        self.status_filter.label()
                    ));
                } else {
                    self.info("showing all cards");
                }
            }
            "status" => {
                if rest.is_empty() {
                    self.error("usage: :status review|marked|test|none");
                    return;
                }
                match Study::parse(rest) {
                    Some(status) => self.set_status(status),
                    None => self.error(format!(
                        "unknown status: {rest} (try: review, marked, test, none)"
                    )),
                }
            }
            "browse" | "b" => {
                if let Some(deck_id) = self.active_deck() {
                    self.enter_browse(deck_id);
                }
            }
            "decks" => {
                self.session = None;
                self.screen = Screen::Decks;
            }
            "stats" => self.open_stats(),
            "help" | "h" => {
                self.help_from = self.screen;
                self.screen = Screen::Help;
                self.help_scroll = 0;
            }
            "add" | "new" => self.open_new_card(),
            "set" => self.cmd_set(rest),
            "forget" | "reset" => {
                let Some(deck) = self.active_deck().and_then(|id| self.db.deck_by_id(id)) else {
                    self.error("no deck selected");
                    return;
                };
                self.confirm = Some(Confirm {
                    prompt: format!(
                        "reset all scheduling for \"{}\" ({} cards)? [y/N]",
                        deck.name,
                        deck.cards.len()
                    ),
                    action: ConfirmAction::ForgetDeck(deck.id),
                });
            }
            "nohl" | "nohlsearch" | "noh" => {
                self.filter.clear();
                self.status_filter = StatusFilter::Off;
                self.refresh_rows();
            }
            other => self.error(format!("unknown command: {other}")),
        }
    }

    fn cmd_set(&mut self, rest: &str) {
        let Some((key, value)) = rest.split_once('=') else {
            self.error("usage: :set new=<n> | :set grading=binary|sm2");
            return;
        };
        let value = value.trim();

        match key.trim() {
            // Collection-wide.
            "grading" | "mode" => {
                let Some(grading) = Grading::parse(value) else {
                    self.error(format!("grading must be binary or sm2, not {value:?}"));
                    return;
                };
                self.checkpoint("the grading mode");
                self.db.grading = grading;
                self.dirty = true;
                self.info(match grading {
                    Grading::Binary => "binary grading — y know, n don't know".to_string(),
                    Grading::Sm2 => "sm2 grading — 1 2 3 4".to_string(),
                });
            }
            // Per-deck.
            "new" | "newperday" => {
                let Ok(amount) = value.parse::<u32>() else {
                    self.error(format!("not a number: {value}"));
                    return;
                };
                let Some(deck_id) = self.active_deck() else {
                    self.error("no deck selected");
                    return;
                };
                self.checkpoint("the new-card limit");
                if let Some(deck) = self.db.deck_by_id_mut(deck_id) {
                    deck.new_per_day = amount;
                }
                self.dirty = true;
                self.info(format!("{amount} new cards per day"));
            }
            other => self.error(format!("unknown setting: {other}")),
        }
    }

    fn cmd_import(&mut self, rest: &str) {
        if rest.is_empty() {
            self.error("usage: :import <file.csv> [deck name]");
            return;
        }
        // `:import path/to/file.csv Some Deck`
        let (path_text, deck_name) = match rest.split_once(char::is_whitespace) {
            Some((path, name)) => (path, name.trim().to_string()),
            None => (rest, String::new()),
        };

        let path = match store::expand_tilde(path_text) {
            Ok(path) => path,
            Err(err) => {
                self.error(err.to_string());
                return;
            }
        };

        let deck_name = if !deck_name.is_empty() {
            deck_name
        } else if let Some(deck) = self.active_deck().and_then(|id| self.db.deck_by_id(id)) {
            deck.name.clone()
        } else {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("imported")
                .to_string()
        };

        // The importer ingests row by row, so a file that goes bad halfway
        // through leaves a half-imported deck behind. Stage the work on a copy
        // and only commit once the whole file has parsed — a failed `:import`
        // must leave the collection exactly as it was, down to not creating
        // the deck the file was aimed at.
        let target = self.db.deck_index(&deck_name);
        let mut staged = match target {
            Some(index) => self.db.decks[index].clone(),
            None => Deck::new(0, deck_name.clone()),
        };

        // `next_card_id` needs `&mut self.db` while the deck is borrowed, so
        // hand the importer a counter it can bump on its own.
        let mut next = self.db.next_card_id();
        let result = store::import_csv(
            &mut staged,
            &mut || {
                next += 1;
                next - 1
            },
            &path,
            self.today,
        );

        match result {
            Ok(report) => {
                // Only once the whole file has parsed: a failed import changes
                // nothing, so it has nothing for `u` to take back.
                self.checkpoint(format!("importing into {deck_name}"));
                let index = match target {
                    Some(index) => {
                        self.db.decks[index] = staged;
                        index
                    }
                    None => {
                        staged.id = self.db.next_deck_id();
                        self.db.decks.push(staged);
                        self.db.decks.len() - 1
                    }
                };
                // Re-seat the counter past whatever the importer consumed.
                self.db.reseat_ids();
                self.decks_sel = index;
                self.dirty = true;
                self.refresh_rows();
                self.info(format!(
                    "{deck_name}: +{} new, {} updated, {} skipped",
                    report.added, report.updated, report.skipped
                ));
                if let Err(err) = self.save() {
                    self.error(format!("imported, but save failed: {err:#}"));
                }
            }
            // `staged` is dropped with every change it accumulated.
            Err(err) => self.error(format!("import failed: {err}")),
        }
    }

    fn cmd_export(&mut self, rest: &str) {
        if rest.is_empty() {
            self.error("usage: :export <file.csv>");
            return;
        }
        let Some(deck) = self.active_deck().and_then(|id| self.db.deck_by_id(id)) else {
            self.error("no deck selected");
            return;
        };
        let path = match store::expand_tilde(rest) {
            Ok(path) => path,
            Err(err) => {
                self.error(err.to_string());
                return;
            }
        };
        match store::export_csv(deck, &path) {
            Ok(count) => self.info(format!("exported {count} cards to {}", path.display())),
            Err(err) => self.error(format!("export failed: {err}")),
        }
    }

    // -- statistics --------------------------------------------------------

    /// Reviews per day over the last `days` days, oldest first.
    pub fn review_history(&self, days: usize) -> Vec<(NaiveDate, u64)> {
        (0..days)
            .rev()
            .map(|back| {
                let date = self.today - Duration::days(back as i64);
                let count = self.db.log.iter().filter(|e| e.date == date).count() as u64;
                (date, count)
            })
            .collect()
    }

    /// Cards falling due on each of the next `days` days.
    pub fn due_forecast(&self, days: usize) -> Vec<(NaiveDate, u64)> {
        (0..days)
            .map(|ahead| {
                let date = self.today + Duration::days(ahead as i64);
                let count = self
                    .db
                    .decks
                    .iter()
                    .flat_map(|d| d.cards.iter())
                    .filter(|c| {
                        !c.suspended && c.state != CardState::New && {
                            if ahead == 0 {
                                c.due <= date
                            } else {
                                c.due == date
                            }
                        }
                    })
                    .count() as u64;
                (date, count)
            })
            .collect()
    }

    /// Share of every answer that was not `Again` — drills, same-session
    /// retries and first sightings of new words included. A measure of how the
    /// sessions went, not of what you remember.
    pub fn answer_accuracy(&self) -> Option<f32> {
        let total = self.db.log.len();
        if total == 0 {
            return None;
        }
        let good = self
            .db
            .log
            .iter()
            .filter(|e| e.grade != Grade::Again.index())
            .count();
        Some(good as f32 / total as f32)
    }

    /// Share of scheduled reviews answered right the first time: the number
    /// that actually says whether the schedule is working.
    ///
    /// Counts one answer per card per day — the first — and leaves out drills,
    /// which ignore due dates by design, and words being seen for the first
    /// time, which is learning rather than remembering. Collections logged
    /// before those two were recorded count as scheduled reviews, which is
    /// what they were for anyone not drilling.
    pub fn review_retention(&self) -> Option<f32> {
        let mut seen: HashSet<(u64, NaiveDate)> = HashSet::new();
        let mut total = 0usize;
        let mut good = 0usize;
        for entry in &self.db.log {
            if entry.drill || entry.learning {
                continue;
            }
            // A retry later the same day is the same question asked twice.
            if !seen.insert((entry.card_id, entry.date)) {
                continue;
            }
            total += 1;
            if entry.grade != Grade::Again.index() {
                good += 1;
            }
        }
        (total > 0).then(|| good as f32 / total as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Card;

    fn key(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
    }

    /// Id of the one deck `app_with_cards` builds. Ids start at 1, so `0`
    /// stays available as "no deck".
    const DECK: u64 = 1;

    /// Draw one frame into a throwaway terminal. The window the list ends up
    /// showing lives in the render, so testing it means rendering.
    fn draw(app: &mut App, width: u16, height: u16) {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| crate::ui::render(frame, app))
            .expect("draw");
    }

    fn app_with_cards(count: usize) -> App {
        let mut db = Db::default();
        let index = db.add_deck("test".into());
        let today = Local::now().date_naive();
        for i in 0..count {
            let id = db.next_card_id();
            db.decks[index].cards.push(Card::new(
                id,
                format!("term{i}"),
                format!("definition {i}"),
                today,
            ));
        }
        assert_eq!(
            db.decks[index].id, DECK,
            "DECK is out of step with add_deck"
        );
        let path =
            std::env::temp_dir().join(format!("gram-test-{}-{}.json", std::process::id(), count));
        let mut app = App::new(db, path);
        app.refresh_rows();
        app
    }

    #[test]
    fn escape_round_trips_newlines_and_backslashes() {
        for original in ["a\nb", "back\\slash", "mix\\n\nreal", "", "trailing\\"] {
            assert_eq!(unescape(&escape(original)), original, "{original:?}");
        }
    }

    #[test]
    fn queue_contains_every_available_card() {
        let mut app = app_with_cards(5);
        app.db.decks[0].new_per_day = 100;
        let queue = app.build_queue(DECK, Scope::Due);
        assert_eq!(queue.len(), 5);
    }

    #[test]
    fn queue_respects_the_daily_new_limit() {
        let mut app = app_with_cards(50);
        app.db.decks[0].new_per_day = 7;
        assert_eq!(app.build_queue(DECK, Scope::Due).len(), 7);
    }

    #[test]
    fn suspended_cards_stay_out_of_the_queue() {
        let mut app = app_with_cards(4);
        app.db.decks[0].new_per_day = 100;
        app.db.decks[0].cards[1].suspended = true;
        assert_eq!(app.build_queue(DECK, Scope::Due).len(), 3);
    }

    #[test]
    fn answering_good_advances_the_queue_and_logs() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        assert_eq!(app.session.as_ref().unwrap().queue.len(), 3);

        app.answer(Grade::Good);
        assert_eq!(app.session.as_ref().unwrap().queue.len(), 2);
        assert_eq!(app.db.log.len(), 1);
        assert_eq!(app.db.decks[0].intro_count, 1);
    }

    #[test]
    fn the_log_records_the_deck_id_not_its_index() {
        let mut app = app_with_cards(2);
        app.db.decks[0].new_per_day = 100;
        app.db.decks[0].id = 77;
        app.start_session(77, Scope::Due);
        app.answer(Grade::Good);
        assert_eq!(app.db.log[0].deck_id, 77);
    }

    #[test]
    fn answering_again_puts_the_card_back() {
        let mut app = app_with_cards(12);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        let first = app.session.as_ref().unwrap().current().unwrap();

        app.answer(Grade::Again);
        let queue = &app.session.as_ref().unwrap().queue;
        assert_eq!(queue.len(), 12, "card returns to the queue");
        assert!(queue.contains(&first));
        assert_ne!(queue.front(), Some(&first), "but not immediately");
    }

    /// Type a `:` command line, keystroke by keystroke.
    fn typed(app: &mut App, text: &str) {
        app.on_key(key(':')).unwrap();
        for ch in text.chars() {
            app.on_key(key(ch)).unwrap();
        }
    }

    fn tab(app: &mut App) {
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
    }

    fn back_tab(app: &mut App) {
        app.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE))
            .unwrap();
    }

    #[test]
    fn tab_completes_a_lone_command_outright() {
        let mut app = app_with_cards(1);
        typed(&mut app, "expo");
        tab(&mut app);
        assert_eq!(
            app.line.value(),
            "export ",
            "and spaces along to the argument"
        );
        assert!(app.completion.is_none(), "nothing left to choose");
    }

    #[test]
    fn tab_fills_the_shared_prefix_then_cycles() {
        let mut app = app_with_cards(1);
        typed(&mut app, "st");

        // stats, status and study all start with st — nothing to fill in yet,
        // so Tab goes straight to offering them.
        tab(&mut app);
        assert_eq!(app.line.value(), "stats");
        let completion = app.completion.as_ref().expect("candidates are on offer");
        assert_eq!(completion.candidates, vec!["stats", "status", "study"]);
        assert_eq!(completion.index, Some(0));

        tab(&mut app);
        assert_eq!(app.line.value(), "status");
        tab(&mut app);
        assert_eq!(app.line.value(), "study");
        tab(&mut app);
        assert_eq!(app.line.value(), "stats", "and round again");

        back_tab(&mut app);
        assert_eq!(app.line.value(), "study", "backwards too");
    }

    #[test]
    fn tab_fills_in_as_far_as_the_candidates_agree() {
        let mut app = app_with_cards(1);
        typed(&mut app, "q");
        tab(&mut app);
        // q, q!, qa, qa! and quit all start with q, so the first press has
        // nothing to add and offers the first instead.
        assert_eq!(app.line.value(), "q");
        assert_eq!(
            app.completion.as_ref().map(|c| c.candidates.clone()),
            Some(vec![
                "q".to_string(),
                "q!".to_string(),
                "qa".to_string(),
                "qa!".to_string(),
                "quit".to_string(),
            ])
        );

        let mut app = app_with_cards(1);
        typed(&mut app, "stud");
        tab(&mut app);
        assert_eq!(app.line.value(), "study ", "one match, taken whole");
    }

    #[test]
    fn tab_completes_arguments_not_just_commands() {
        let mut app = app_with_cards(1);
        typed(&mut app, "status re");
        tab(&mut app);
        assert_eq!(app.line.value(), "status review ");

        let mut app = app_with_cards(1);
        typed(&mut app, "set grading=b");
        tab(&mut app);
        assert_eq!(app.line.value(), "set grading=binary ");
    }

    #[test]
    fn an_unfinished_word_does_not_get_a_space() {
        let mut app = app_with_cards(1);
        typed(&mut app, "set g");
        tab(&mut app);
        assert_eq!(
            app.line.value(),
            "set grading=",
            "the value still has to be typed, so no space lands in between"
        );

        // And a second Tab carries on into the values.
        tab(&mut app);
        assert_eq!(app.line.value(), "set grading=binary");
    }

    #[test]
    fn typing_ends_the_completion() {
        let mut app = app_with_cards(1);
        typed(&mut app, "st");
        tab(&mut app);
        assert!(app.completion.is_some());

        app.on_key(key('a')).unwrap();
        assert!(app.completion.is_none(), "the menu goes away");
        assert_eq!(app.line.value(), "statsa", "and the key is just typed");
    }

    #[test]
    fn a_completed_command_runs() {
        let mut app = app_with_cards(1);
        typed(&mut app, "brow");
        tab(&mut app);
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.screen, Screen::Browse);
        assert!(app.completion.is_none());
    }

    #[test]
    fn tab_does_nothing_for_an_unknown_command() {
        let mut app = app_with_cards(1);
        typed(&mut app, "zzz");
        tab(&mut app);
        assert_eq!(app.line.value(), "zzz");
        assert!(app.completion.is_none());
    }

    #[test]
    fn search_is_not_completed() {
        let mut app = app_with_cards(1);
        app.enter_browse(DECK);
        app.on_key(key('/')).unwrap();
        for ch in "st".chars() {
            app.on_key(key(ch)).unwrap();
        }
        tab(&mut app);
        assert_eq!(app.line.value(), "st", "a search is free text");
        assert!(app.completion.is_none());
    }

    #[test]
    fn saving_over_someone_elses_write_keeps_their_version() {
        let dir = std::env::temp_dir().join(format!("gram-conflict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("collection.json");

        // This app loaded when there was no collection at this path yet.
        let mut app = app_with_cards(2);
        app.path = path.clone();
        app.written = store::modified(&path);
        assert_eq!(app.written, None, "nothing written there yet");

        // Something else writes one underneath us — `gram import` in another
        // terminal, say.
        let theirs = r#"{"decks":[{"id":9,"name":"theirs","cards":[]}]}"#;
        std::fs::write(&path, theirs).unwrap();

        let aside = app.save().unwrap().expect("the clash is noticed");
        assert_eq!(
            std::fs::read_to_string(&aside).unwrap(),
            theirs,
            "their collection is kept, not lost"
        );
        let ours = store::load_from(&path).unwrap();
        assert_eq!(ours.decks[0].name, "test", "and ours is what is loaded now");

        // Saving again is not a conflict: the last write was our own.
        assert!(app.save().unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Keeping their version is the price of overwriting it. If the copy
    /// cannot be made, refusing to save loses nothing — the session is still
    /// here to retry — whereas writing anyway loses their collection.
    #[test]
    #[cfg(unix)]
    fn a_save_that_cannot_keep_their_version_refuses_to_overwrite() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("gram-noconflict-{}", std::process::id()));
        let writable = |mode: u32| {
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode));
        };
        // An earlier failure can leave this directory unwritable, which would
        // make every later run fail for the wrong reason.
        writable(0o700);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("collection.json");

        let mut app = app_with_cards(2);
        app.path = path.clone();
        app.written = store::modified(&path);

        let theirs = r#"{"decks":[{"id":9,"name":"theirs","cards":[]}]}"#;
        std::fs::write(&path, theirs).unwrap();

        // A directory nothing can be created in is the simplest way to make
        // the backup fail. Root ignores the mode, so check before trusting it.
        writable(0o500);
        if std::fs::write(dir.join("probe"), "").is_ok() {
            writable(0o700);
            let _ = std::fs::remove_dir_all(&dir);
            return; // running as root; there is nothing to test
        }

        // Nothing may panic while the directory is locked, or the next run
        // inherits it — so the save happens here and the asserts come after.
        app.dirty = true;
        let outcome = app.save();
        writable(0o700);

        let err = outcome.expect_err("the save must not go through");
        assert!(
            format!("{err:#}").contains("could not be backed up"),
            "the error says why: {err:#}"
        );
        assert!(app.dirty, "and the session still knows it is unsaved");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            theirs,
            "their collection is untouched"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_puts_a_kept_copy_back() {
        let dir = std::env::temp_dir().join(format!("gram-apprestore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("collection.json");

        let mut app = app_with_cards(3);
        app.path = path.clone();
        app.written = None;

        // Something else writes a collection, so saving keeps theirs aside.
        let mut theirs = Db::default();
        let index = theirs.add_deck("theirs".into());
        theirs.decks[index].cards.push(Card::new(
            1,
            "abash".into(),
            "to embarrass".into(),
            app.today,
        ));
        store::save_to(&theirs, &path).unwrap();
        let aside = app.save().unwrap().expect("a conflict");

        // With a session open and ids in flight, to prove a restore clears it.
        app.start_session(DECK, Scope::Due);
        assert!(app.session.is_some());

        // `:restore` on its own only reports what there is.
        typed(&mut app, "restore");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(app.confirm.is_none(), "listing asks nothing");
        assert!(
            app.status.text.contains(&store::backup_label(&aside)),
            "it names the copy: {:?}",
            app.status.text
        );
        assert_eq!(app.db.decks[0].name, "test", "and changes nothing");

        // `:restore 1` asks first, and n is a no.
        typed(&mut app, "restore 1");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(app.confirm.is_some(), "a restore is confirmed");
        app.on_key(key('n')).unwrap();
        assert_eq!(app.db.decks[0].name, "test", "declined, nothing moved");

        typed(&mut app, "restore 1");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        app.on_key(key('y')).unwrap();

        assert_eq!(app.db.decks.len(), 1);
        assert_eq!(app.db.decks[0].name, "theirs", "their deck is live now");
        assert_eq!(app.db.decks[0].cards[0].term, "abash");
        assert!(app.session.is_none(), "a session over other cards is ended");
        assert_eq!(app.screen, Screen::Decks);
        assert!(!app.dirty, "the restored collection is what is on disk");
        assert_eq!(
            store::load_from(&path).unwrap().decks[0].name,
            "theirs",
            "and it was written, not just loaded"
        );

        // Our own collection was kept on the way out, so this is reversible.
        let kept = store::conflict_copies(&path);
        assert!(
            kept.iter()
                .any(|p| store::load_from(p)
                    .is_ok_and(|db| db.decks.iter().any(|d| d.name == "test"))),
            "what the restore displaced is still there: {kept:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_session_checkpoints_as_it_goes() {
        let mut app = app_with_cards(30);
        app.path =
            std::env::temp_dir().join(format!("gram-checkpoint-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&app.path);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);

        for _ in 0..CHECKPOINT_EVERY - 1 {
            app.answer(Grade::Good);
        }
        assert!(app.dirty, "nothing written yet");
        assert!(!app.path.exists());

        app.answer(Grade::Good);
        assert!(!app.dirty, "the tenth answer checkpoints");
        let saved = store::load_from(&app.path).expect("a readable collection");
        assert_eq!(
            saved.log.len(),
            CHECKPOINT_EVERY as usize,
            "every answer so far is on disk"
        );
        let _ = std::fs::remove_file(&app.path);
    }

    #[test]
    fn retention_leaves_out_drills_retries_and_first_sightings() {
        let mut app = app_with_cards(4);
        app.db.decks[0].new_per_day = 100;

        // Learn all four, which is not retention: nothing is being remembered.
        app.start_session(DECK, Scope::Due);
        for _ in 0..4 {
            app.answer(Grade::Good);
        }
        assert_eq!(app.review_retention(), None, "only new cards so far");
        assert_eq!(app.answer_accuracy(), Some(1.0));

        // A day later they come round as scheduled reviews: one Again, then
        // the retry, then two more Goods.
        app.today += Duration::days(1);
        for card in &mut app.db.decks[0].cards {
            card.due = app.today;
        }
        app.start_session(DECK, Scope::Due);
        app.answer(Grade::Again);
        app.answer(Grade::Good);
        app.answer(Grade::Good);
        app.answer(Grade::Good);

        // Four cards, four first answers, one of them wrong. The retry on the
        // lapsed card does not get a second vote.
        assert_eq!(app.review_retention(), Some(0.75));

        // A drill the same day does not move retention, but does move accuracy.
        let before = app.review_retention();
        app.db.decks[0].cards[0].status = Study::Marked;
        app.start_session(DECK, Scope::Flagged(None));
        app.answer(Grade::Again);
        assert_eq!(app.review_retention(), before, "drills are not retention");
        assert!(
            app.answer_accuracy().unwrap() < 0.8,
            "but every answer counts towards accuracy"
        );
    }

    #[test]
    fn an_older_log_counts_as_scheduled_reviews() {
        let mut app = app_with_cards(2);
        // Entries written before drills and new cards were recorded.
        for (card_id, grade) in [(1, Grade::Good.index()), (2, Grade::Again.index())] {
            app.db.log.push(ReviewLog {
                date: app.today,
                deck_id: DECK,
                card_id,
                grade,
                drill: false,
                learning: false,
            });
        }
        assert_eq!(app.review_retention(), Some(0.5));
    }

    #[test]
    fn a_retry_does_not_count_as_progress() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        let first = app.session.as_ref().unwrap().current().unwrap();

        // Again, Good, Good: three answers, but only two cards seen off.
        app.answer(Grade::Again);
        app.answer(Grade::Good);
        app.answer(Grade::Good);

        let session = app.session.as_ref().unwrap();
        assert_eq!(session.answered, 3, "every answer is still counted");
        assert_eq!(session.completed, 2, "the retried card is not done yet");
        assert_eq!(session.retrying.len(), 1);
        assert!(session.retrying.contains(&first));
        assert!(
            session.completed < session.initial,
            "the bar cannot read 100% with a card still in the queue"
        );

        // Finishing the retry completes it and clears the retry count.
        while app.session.as_ref().is_some_and(|s| !s.queue.is_empty()) {
            app.answer(Grade::Good);
        }
        assert!(app.session.is_none(), "the session ended");
    }

    #[test]
    fn undo_puts_the_retry_count_back() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        let first = app.session.as_ref().unwrap().current().unwrap();

        app.answer(Grade::Again);
        assert_eq!(app.session.as_ref().unwrap().retrying.len(), 1);
        app.undo_answer();
        let session = app.session.as_ref().unwrap();
        assert!(session.retrying.is_empty(), "the retry is undone too");
        assert_eq!(session.completed, 0);

        // And the other way round: undoing a completion un-completes it
        // without inventing a retry.
        app.answer(Grade::Good);
        assert_eq!(app.session.as_ref().unwrap().completed, 1);
        app.undo_answer();
        let session = app.session.as_ref().unwrap();
        assert_eq!(session.completed, 0);
        assert!(session.retrying.is_empty());
        assert_eq!(session.current(), Some(first), "and it is up again");
    }

    #[test]
    fn a_card_retried_twice_is_counted_once() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);

        // Again on the first card, then Again on it again when it comes back.
        app.answer(Grade::Again);
        app.answer(Grade::Good);
        app.answer(Grade::Good);
        app.answer(Grade::Again);
        let session = app.session.as_ref().unwrap();
        assert_eq!(session.retrying.len(), 1, "one card is awaiting another go");
        assert_eq!(session.completed, 2);
    }

    #[test]
    fn suspending_mid_session_counts_as_dealt_with() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);

        app.on_key(key('x')).unwrap();
        let session = app.session.as_ref().unwrap();
        assert_eq!(session.completed, 1, "it left the queue for good");
        assert_eq!(session.queue.len(), 2);
    }

    #[test]
    fn j_and_k_scroll_the_card_rather_than_the_deck_list() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        let before = app.decks_sel;

        app.on_key(key('j')).unwrap();
        assert_eq!(app.card_scroll, 1);
        app.on_key(key('k')).unwrap();
        assert_eq!(app.card_scroll, 0);
        app.on_key(key('k')).unwrap();
        assert_eq!(app.card_scroll, 0, "and does not go negative");
        assert_eq!(app.decks_sel, before, "the deck cursor stays put");
    }

    #[test]
    fn turning_a_card_over_starts_at_the_top() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);

        app.on_key(key(' ')).unwrap();
        app.on_key(key('j')).unwrap();
        app.on_key(key('j')).unwrap();
        assert_eq!(app.card_scroll, 2);

        app.answer(Grade::Good);
        assert_eq!(app.card_scroll, 0, "the next card starts at the top");

        app.on_key(key(' ')).unwrap();
        app.on_key(key('j')).unwrap();
        app.undo_answer();
        assert_eq!(app.card_scroll, 0, "and so does an undone one");
    }

    #[test]
    fn undo_restores_scheduling_and_queue_position() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        let first = app.session.as_ref().unwrap().current().unwrap();
        let before = app.db.decks[0].card(first).unwrap().clone();

        app.answer(Grade::Good);
        app.undo_answer();

        let after = app.db.decks[0].card(first).unwrap();
        assert_eq!(after.state, before.state);
        assert_eq!(after.interval, before.interval);
        assert_eq!(app.session.as_ref().unwrap().current(), Some(first));
        assert_eq!(app.db.log.len(), 0);
        assert_eq!(app.db.decks[0].intro_count, 0);
    }

    #[test]
    fn undo_after_again_removes_the_requeued_copy() {
        let mut app = app_with_cards(12);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        let first = app.session.as_ref().unwrap().current().unwrap();

        app.answer(Grade::Again);
        app.undo_answer();

        let queue = &app.session.as_ref().unwrap().queue;
        assert_eq!(queue.len(), 12);
        assert_eq!(queue.iter().filter(|id| **id == first).count(), 1);
        assert_eq!(queue.front(), Some(&first));
    }

    #[test]
    fn session_ends_when_the_queue_empties() {
        let mut app = app_with_cards(1);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        app.answer(Grade::Good);
        assert!(app.session.is_none());
        assert_eq!(app.screen, Screen::Decks);
    }

    #[test]
    fn binary_mode_marks_a_word_you_miss() {
        let mut app = app_with_cards(4);
        app.db.grading = Grading::Binary;
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        let id = app.session.as_ref().unwrap().current().unwrap();

        app.answer(Grade::Again);
        assert!(app.db.decks[0].card(id).unwrap().is_flagged());
    }

    #[test]
    fn sm2_mode_leaves_marks_alone() {
        let mut app = app_with_cards(4);
        app.db.grading = Grading::Sm2;
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        let id = app.session.as_ref().unwrap().current().unwrap();

        app.answer(Grade::Again);
        assert!(!app.db.decks[0].card(id).unwrap().is_flagged());
    }

    #[test]
    fn knowing_a_word_does_not_mark_it() {
        let mut app = app_with_cards(4);
        app.db.grading = Grading::Binary;
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        let id = app.session.as_ref().unwrap().current().unwrap();

        app.answer(Grade::Good);
        assert!(!app.db.decks[0].card(id).unwrap().is_flagged());
    }

    #[test]
    fn undo_clears_a_mark_the_miss_added() {
        let mut app = app_with_cards(4);
        app.db.grading = Grading::Binary;
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        let id = app.session.as_ref().unwrap().current().unwrap();

        app.answer(Grade::Again);
        assert!(app.db.decks[0].card(id).unwrap().is_flagged());

        app.undo_answer();
        assert!(!app.db.decks[0].card(id).unwrap().is_flagged());
    }

    #[test]
    fn a_marked_drill_ignores_due_dates() {
        let mut app = app_with_cards(6);
        // Everything is new and none of it is due, so a normal session with a
        // zero new-card ration would be empty.
        app.db.decks[0].new_per_day = 0;
        assert!(app.build_queue(DECK, Scope::Due).is_empty());

        app.db.decks[0].cards[1].status = Study::Marked;
        app.db.decks[0].cards[4].status = Study::Marked;
        let queue = app.build_queue(DECK, Scope::Flagged(None));
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn a_marked_drill_skips_suspended_cards() {
        let mut app = app_with_cards(4);
        for card in &mut app.db.decks[0].cards {
            card.status = Study::Marked;
        }
        app.db.decks[0].cards[0].suspended = true;
        assert_eq!(app.build_queue(DECK, Scope::Flagged(None)).len(), 3);
    }

    #[test]
    fn m_cycles_the_status_in_the_browser() {
        let mut app = app_with_cards(3);
        app.enter_browse(DECK);
        app.browse_sel = 2;

        let status = |app: &App| app.db.decks[0].cards[2].status;
        assert_eq!(status(&app), Study::None);

        for expected in [Study::Review, Study::Marked, Study::Test, Study::None] {
            app.on_key(key('m')).unwrap();
            assert_eq!(status(&app), expected, "m steps forward through the cycle");
        }
    }

    #[test]
    fn z_hides_every_meaning_and_shows_them_again() {
        let mut app = app_with_cards(3);
        app.enter_browse(DECK);

        app.on_key(key('z')).unwrap();
        assert!(app.hide_meanings, "z hides");
        assert!((0..3).all(|row| app.meaning_hidden(row)), "the whole list");

        app.on_key(key('z')).unwrap();
        assert!(!app.hide_meanings, "z again shows");
        assert!((0..3).all(|row| !app.meaning_hidden(row)));
    }

    #[test]
    fn space_uncovers_only_the_selected_word() {
        let mut app = app_with_cards(3);
        app.enter_browse(DECK);
        app.on_key(key('z')).unwrap();
        app.on_key(key(' ')).unwrap();

        assert!(!app.meaning_hidden(0), "the one under the cursor");
        assert!(app.meaning_hidden(1), "and no other");
        assert!(app.hide_meanings, "space does not leave the mode");
    }

    #[test]
    fn moving_on_covers_the_word_space_uncovered() {
        let mut app = app_with_cards(3);
        app.enter_browse(DECK);
        app.on_key(key('z')).unwrap();
        app.on_key(key(' ')).unwrap();
        app.on_key(key('j')).unwrap();

        assert!(app.meaning_hidden(1), "the next word starts covered");
        assert!(app.meaning_hidden(0), "and the last one is covered again");
    }

    #[test]
    fn space_does_nothing_while_the_meanings_are_showing() {
        let mut app = app_with_cards(3);
        app.enter_browse(DECK);
        app.on_key(key(' ')).unwrap();

        assert!(!app.peek, "nothing to uncover");
        assert!(!app.hide_meanings);
    }

    #[test]
    fn ctrl_m_cycles_the_other_way() {
        let mut app = app_with_cards(3);
        app.enter_browse(DECK);
        app.browse_sel = 1;

        let back = KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL);
        app.on_key(back).unwrap();
        assert_eq!(app.db.decks[0].cards[1].status, Study::Test);
        app.on_key(back).unwrap();
        assert_eq!(app.db.decks[0].cards[1].status, Study::Marked);
    }

    #[test]
    fn m_cycles_the_status_while_reviewing() {
        let mut app = app_with_cards(4);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        let id = app.session.as_ref().unwrap().current().unwrap();

        app.on_key(key('m')).unwrap();
        assert_eq!(app.db.decks[0].card(id).unwrap().status, Study::Review);
        app.on_key(key('m')).unwrap();
        assert_eq!(app.db.decks[0].card(id).unwrap().status, Study::Marked);
    }

    #[test]
    fn status_command_sets_one_outright() {
        let mut app = app_with_cards(3);
        app.enter_browse(DECK);
        app.browse_sel = 0;

        app.run_command("status test");
        assert_eq!(app.db.decks[0].cards[0].status, Study::Test);

        app.run_command("status none");
        assert_eq!(app.db.decks[0].cards[0].status, Study::None);

        app.run_command("status nonsense");
        assert!(app.status.error);
        assert_eq!(
            app.db.decks[0].cards[0].status,
            Study::None,
            "a bad value changes nothing"
        );
    }

    #[test]
    fn a_drill_can_target_one_status() {
        let mut app = app_with_cards(6);
        app.db.decks[0].cards[0].status = Study::Review;
        app.db.decks[0].cards[1].status = Study::Test;
        app.db.decks[0].cards[2].status = Study::Test;

        assert_eq!(app.build_queue(DECK, Scope::Flagged(None)).len(), 3);
        assert_eq!(
            app.build_queue(DECK, Scope::Flagged(Some(Study::Test)))
                .len(),
            2
        );
        assert_eq!(
            app.build_queue(DECK, Scope::Flagged(Some(Study::Marked)))
                .len(),
            0
        );
    }

    #[test]
    fn the_browser_can_filter_to_one_status() {
        let mut app = app_with_cards(8);
        app.enter_browse(DECK);
        app.db.decks[0].cards[2].status = Study::Review;
        app.db.decks[0].cards[5].status = Study::Test;

        app.run_command("marked");
        assert_eq!(
            app.rows,
            vec![2, 5],
            "bare :marked shows everything flagged"
        );

        app.run_command("marked test");
        assert_eq!(app.rows, vec![5]);

        app.run_command("noh");
        assert_eq!(app.rows.len(), 8);
    }

    #[test]
    fn a_miss_never_overwrites_a_status_you_set() {
        let mut app = app_with_cards(4);
        app.db.grading = Grading::Binary;
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        let id = app.session.as_ref().unwrap().current().unwrap();
        app.db
            .deck_by_id_mut(DECK)
            .unwrap()
            .card_mut(id)
            .unwrap()
            .status = Study::Test;

        app.answer(Grade::Again);
        assert_eq!(
            app.db.decks[0].card(id).unwrap().status,
            Study::Test,
            "your own label wins over the automatic mark"
        );
    }

    #[test]
    fn an_old_collection_keeps_its_marks() {
        // Written before `status` existed: a plain `"marked": true`.
        let json = r#"{"decks":[{"id":1,"name":"d","cards":[
            {"id":1,"term":"a","definition":"x","due":"2026-01-01","marked":true},
            {"id":2,"term":"b","definition":"y","due":"2026-01-01","marked":false}
        ]}]}"#;
        let mut db: Db = serde_json::from_str(json).unwrap();
        db.migrate();

        assert_eq!(db.decks[0].cards[0].status, Study::Marked);
        assert_eq!(db.decks[0].cards[1].status, Study::None);

        // The status is written in the new shape, and the legacy boolean is
        // gone once it has been folded in.
        let round_trip = serde_json::to_string(&db).unwrap();
        assert!(round_trip.contains(r#""status":"marked""#), "{round_trip}");
        assert!(!round_trip.contains(r#""marked":"#), "{round_trip}");
    }

    #[test]
    fn marked_filter_narrows_the_browser() {
        let mut app = app_with_cards(10);
        app.enter_browse(DECK);
        app.db.decks[0].cards[3].status = Study::Marked;
        app.db.decks[0].cards[7].status = Study::Marked;

        app.run_command("marked");
        assert_eq!(app.status_filter, StatusFilter::Any);
        assert_eq!(app.rows, vec![3, 7]);

        app.run_command("noh");
        assert_eq!(app.status_filter, StatusFilter::Off);
        assert_eq!(app.rows.len(), 10);
    }

    #[test]
    fn marked_filter_composes_with_search() {
        let mut app = app_with_cards(20);
        app.enter_browse(DECK);
        for i in [1, 2, 11] {
            app.db.decks[0].cards[i].status = Study::Marked;
        }
        app.filter = "term1".to_string();
        app.status_filter = StatusFilter::Any;
        app.refresh_rows();
        // term1 and term11 match the text; term2 is marked but does not.
        assert_eq!(app.rows, vec![1, 11]);
    }

    #[test]
    fn study_marked_starts_a_marked_session() {
        let mut app = app_with_cards(5);
        app.db.decks[0].cards[0].status = Study::Marked;
        app.run_command("study marked");

        let session = app.session.as_ref().expect("session started");
        // `:study marked` targets that one status; `M` drills everything flagged.
        assert_eq!(session.scope, Scope::Flagged(Some(Study::Marked)));
        assert_eq!(session.queue.len(), 1);
    }

    #[test]
    fn study_marked_with_nothing_marked_reports_it() {
        let mut app = app_with_cards(5);
        app.run_command("study marked");
        assert!(app.session.is_none());
        assert!(app.status.text.contains("no marked words"));
    }

    #[test]
    fn set_grading_switches_modes() {
        let mut app = app_with_cards(1);
        app.run_command("set grading=sm2");
        assert_eq!(app.db.grading, Grading::Sm2);

        app.run_command("set grading=binary");
        assert_eq!(app.db.grading, Grading::Binary);

        app.run_command("set grading=nonsense");
        assert!(app.status.error);
        assert_eq!(
            app.db.grading,
            Grading::Binary,
            "a bad value changes nothing"
        );
    }

    #[test]
    fn y_and_n_grade_only_after_the_reveal() {
        let mut app = app_with_cards(4);
        app.db.grading = Grading::Binary;
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);

        app.on_key(key('n')).unwrap();
        assert_eq!(app.session.as_ref().unwrap().answered, 0);
        assert!(app.session.as_ref().unwrap().revealed, "it reveals instead");

        app.on_key(key('n')).unwrap();
        assert_eq!(app.session.as_ref().unwrap().answered, 1);
    }

    #[test]
    fn y_grades_good_and_n_grades_again() {
        let mut app = app_with_cards(4);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);

        app.on_key(key(' ')).unwrap();
        app.on_key(key('y')).unwrap();
        assert_eq!(app.db.log[0].grade, Grade::Good.index());

        app.on_key(key(' ')).unwrap();
        app.on_key(key('n')).unwrap();
        assert_eq!(app.db.log[1].grade, Grade::Again.index());
    }

    #[test]
    fn dd_then_u_restores_the_card_in_place() {
        let mut app = app_with_cards(3);
        app.enter_browse(DECK);
        app.browse_sel = 1;

        app.on_key(key('d')).unwrap();
        app.on_key(key('d')).unwrap();
        assert_eq!(app.db.decks[0].cards.len(), 2);

        app.on_key(key('u')).unwrap();
        assert_eq!(app.db.decks[0].cards.len(), 3);
        assert_eq!(app.db.decks[0].cards[1].term, "term1");
    }

    #[test]
    fn counts_prefix_motions() {
        let mut app = app_with_cards(20);
        app.enter_browse(DECK);
        app.on_key(key('5')).unwrap();
        app.on_key(key('j')).unwrap();
        assert_eq!(app.browse_sel, 5);
    }

    #[test]
    fn gg_and_g_jump_to_the_ends() {
        let mut app = app_with_cards(20);
        app.enter_browse(DECK);
        app.on_key(key('G')).unwrap();
        assert_eq!(app.browse_sel, 19);

        app.on_key(key('g')).unwrap();
        app.on_key(key('g')).unwrap();
        assert_eq!(app.browse_sel, 0);
    }

    #[test]
    fn search_filters_rows() {
        let mut app = app_with_cards(15);
        app.enter_browse(DECK);
        app.on_key(key('/')).unwrap();
        assert_eq!(app.mode, Mode::Search);
        for ch in "term1".chars() {
            app.on_key(key(ch)).unwrap();
        }
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();

        // term1, term10..term14
        assert_eq!(app.rows.len(), 6);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn colon_command_creates_a_deck() {
        let mut app = app_with_cards(1);
        app.on_key(key(':')).unwrap();
        for ch in "deck greek".chars() {
            app.on_key(key(ch)).unwrap();
        }
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.db.decks.len(), 2);
        assert_eq!(app.db.decks[1].name, "greek");
    }

    #[test]
    fn editor_writes_a_new_card() {
        let mut app = app_with_cards(1);
        app.enter_browse(DECK);
        app.on_key(key('a')).unwrap();
        assert_eq!(app.mode, Mode::Insert);

        for ch in "zeal".chars() {
            app.on_key(key(ch)).unwrap();
        }
        // Tab past pron and pos to the definition.
        for _ in 0..3 {
            app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
                .unwrap();
        }
        for ch in "eagerness".chars() {
            app.on_key(key(ch)).unwrap();
        }
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        app.on_key(key('Z')).unwrap();
        app.on_key(key('Z')).unwrap();

        assert!(app.editor.is_none());
        let card = app.db.decks[0].cards.last().unwrap();
        assert_eq!(card.term, "zeal");
        assert_eq!(card.definition, "eagerness");
    }

    #[test]
    fn editor_refuses_a_card_without_a_definition() {
        let mut app = app_with_cards(1);
        app.enter_browse(DECK);
        app.open_new_card();
        app.field_mut(|f| f.insert_str("lonely"));
        assert!(!app.commit_editor());
        assert!(app.editor.is_some(), "editor stays open on error");
        assert!(app.status.error);
    }

    #[test]
    fn review_space_reveals_then_grades() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);

        app.on_key(key(' ')).unwrap();
        assert!(app.session.as_ref().unwrap().revealed);

        app.on_key(key(' ')).unwrap();
        assert!(!app.session.as_ref().unwrap().revealed);
        assert_eq!(app.session.as_ref().unwrap().answered, 1);
    }

    #[test]
    fn a_count_survives_the_first_key_of_an_operator() {
        // `5G` and `5gg` both used to lose the count: it was consumed before
        // the operator's second keystroke arrived.
        let mut app = app_with_cards(21);
        app.enter_browse(DECK);
        app.on_key(key('5')).unwrap();
        app.on_key(key('G')).unwrap();
        assert_eq!(app.browse_sel, 4, "5G selects row 5");

        app.on_key(key('1')).unwrap();
        app.on_key(key('2')).unwrap();
        app.on_key(key('g')).unwrap();
        app.on_key(key('g')).unwrap();
        assert_eq!(app.browse_sel, 11, "12gg selects row 12");
    }

    #[test]
    fn bare_g_still_jumps_to_the_last_row() {
        let mut app = app_with_cards(9);
        app.enter_browse(DECK);
        app.on_key(key('G')).unwrap();
        assert_eq!(app.browse_sel, 8);
    }

    /// Height 15: a 1-line header, a 1-line status bar, and a panel whose
    /// border and column header leave ten rows for cards.
    const ROWS_VISIBLE: usize = 10;

    #[test]
    fn going_back_up_the_browser_does_not_keep_the_cursor_at_the_bottom() {
        let mut app = app_with_cards(30);
        app.enter_browse(DECK);
        draw(&mut app, 100, 15);
        assert_eq!(app.browse_off, 0, "a fresh list starts at the top");

        // Down past the window: it scrolls, and the cursor rides the last row.
        app.on_key(key('2')).unwrap();
        app.on_key(key('0')).unwrap();
        app.on_key(key('j')).unwrap();
        draw(&mut app, 100, 15);
        assert_eq!(app.browse_sel, 20);
        assert_eq!(app.browse_off, 20 + 1 - ROWS_VISIBLE);

        // Back up five rows. The cursor is still inside the window, so the
        // window stays where it is instead of redrawing from the top and
        // dragging the cursor back down to the bottom row.
        for _ in 0..5 {
            app.on_key(key('k')).unwrap();
        }
        draw(&mut app, 100, 15);
        assert_eq!(app.browse_sel, 15);
        assert_eq!(app.browse_off, 11, "the window did not follow");

        // Keep going and it scrolls again, now from the top row.
        for _ in 0..5 {
            app.on_key(key('k')).unwrap();
        }
        draw(&mut app, 100, 15);
        assert_eq!(app.browse_sel, 10);
        assert_eq!(app.browse_off, 10);
    }

    #[test]
    fn the_deck_list_keeps_its_window_too() {
        let mut app = app_with_cards(1);
        for i in 0..30 {
            app.db.add_deck(format!("deck{i}"));
        }
        app.on_key(key('G')).unwrap();
        draw(&mut app, 100, 15);
        let last = app.db.decks.len() - 1;
        assert_eq!(app.decks_sel, last);
        assert_eq!(app.decks_off, last + 1 - ROWS_VISIBLE);

        app.on_key(key('k')).unwrap();
        draw(&mut app, 100, 15);
        assert_eq!(app.decks_off, last + 1 - ROWS_VISIBLE, "unchanged");
    }

    #[test]
    fn a_count_no_other_key_uses_is_discarded() {
        let mut app = app_with_cards(12);
        app.enter_browse(DECK);
        app.on_key(key('7')).unwrap();
        app.on_key(key('m')).unwrap(); // mark takes no count
        assert!(
            app.count.is_empty(),
            "a stale count would affect the next motion"
        );
        app.on_key(key('j')).unwrap();
        assert_eq!(app.browse_sel, 1);
    }

    #[test]
    fn qa_saves_rather_than_discarding_unsaved_answers() {
        let mut app = app_with_cards(31);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        app.on_key(key(' ')).unwrap();
        app.on_key(key('y')).unwrap();
        assert!(app.dirty);

        app.run_command("qa");
        assert!(app.quit);
        assert!(!app.dirty, ":qa must not throw the session away");
        assert!(app.path.exists(), "the collection was written");
        std::fs::remove_file(&app.path).ok();
    }

    #[test]
    fn qa_bang_asks_before_discarding() {
        let mut app = app_with_cards(32);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);
        app.on_key(key(' ')).unwrap();
        app.on_key(key('y')).unwrap();

        app.run_command("qa!");
        assert!(!app.quit, "it waits for the confirmation");
        assert!(app.confirm.is_some());

        app.on_key(key('y')).unwrap();
        assert!(app.quit);
    }

    #[test]
    fn deleting_a_deck_drops_every_handle_into_it() {
        let mut app = app_with_cards(4);
        app.db.decks[0].new_per_day = 100;
        app.enter_browse(DECK);
        app.on_key(key('d')).unwrap();
        app.on_key(key('d')).unwrap();
        app.start_session(DECK, Scope::Due);
        assert!(app.session.is_some());

        app.screen = Screen::Decks;
        app.decks_sel = 0;
        app.on_key(key('d')).unwrap();
        app.on_key(key('d')).unwrap();
        app.on_key(key('y')).unwrap();

        assert!(app.db.decks.is_empty());
        assert!(app.session.is_none(), "the session cannot outlive its deck");
        assert_eq!(app.browse_deck, 0);

        // The deck is gone, but not beyond recall: `u` is the last word on
        // every change, a deleted deck included.
        app.on_key(key('u')).unwrap();
        assert_eq!(app.db.decks.len(), 1, "u brings the deck back");
        assert_eq!(app.db.decks[0].cards.len(), 3, "as it was when it went");
    }

    #[test]
    fn u_walks_back_through_every_kind_of_change() {
        let mut app = app_with_cards(3);
        app.enter_browse(DECK);

        app.on_key(key('x')).unwrap(); // suspend a card
        app.on_key(key('m')).unwrap(); // give it a status
        app.run_command("deck second"); // and make a deck
        assert_eq!(app.db.decks.len(), 2);

        app.on_key(key('u')).unwrap();
        assert_eq!(app.db.decks.len(), 1, "the deck goes");
        app.on_key(key('u')).unwrap();
        assert_eq!(
            app.db.decks[0].cards[0].status,
            Study::None,
            "then the status"
        );
        app.on_key(key('u')).unwrap();
        assert!(!app.db.decks[0].cards[0].suspended, "then the suspend");

        app.on_key(key('u')).unwrap();
        assert!(app.status.error, "and then there is nothing left to undo");
        assert_eq!(app.status.text, "already at oldest change");
    }

    #[test]
    fn ctrl_r_puts_back_what_u_took() {
        let mut app = app_with_cards(3);
        app.enter_browse(DECK);
        app.on_key(key('d')).unwrap();
        app.on_key(key('d')).unwrap();
        assert_eq!(app.db.decks[0].cards.len(), 2);

        app.on_key(key('u')).unwrap();
        assert_eq!(app.db.decks[0].cards.len(), 3, "u restores the card");

        let redo = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
        app.on_key(redo).unwrap();
        assert_eq!(app.db.decks[0].cards.len(), 2, "Ctrl-r deletes it again");

        app.on_key(redo).unwrap();
        assert!(app.status.error);
        assert_eq!(app.status.text, "already at newest change");
    }

    #[test]
    fn a_change_after_an_undo_closes_off_the_redo() {
        let mut app = app_with_cards(3);
        app.enter_browse(DECK);
        app.on_key(key('d')).unwrap();
        app.on_key(key('d')).unwrap();
        app.on_key(key('u')).unwrap();

        // Vim's rule: doing something else makes the undone change unreachable.
        app.on_key(key('x')).unwrap();
        let redo = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
        app.on_key(redo).unwrap();
        assert_eq!(
            app.db.decks[0].cards.len(),
            3,
            "the delete does not come back"
        );
        assert_eq!(app.status.text, "already at newest change");
    }

    #[test]
    fn a_count_undoes_that_many_changes() {
        let mut app = app_with_cards(4);
        app.enter_browse(DECK);
        for _ in 0..3 {
            app.on_key(key('d')).unwrap();
            app.on_key(key('d')).unwrap();
        }
        assert_eq!(app.db.decks[0].cards.len(), 1);

        app.on_key(key('3')).unwrap();
        app.on_key(key('u')).unwrap();
        assert_eq!(
            app.db.decks[0].cards.len(),
            4,
            "3u takes back three deletes"
        );
        assert!(app.status.text.contains("3 changes"), "{}", app.status.text);
    }

    #[test]
    fn a_refused_change_leaves_nothing_to_undo() {
        let mut app = app_with_cards(3);
        app.enter_browse(DECK);

        app.run_command("deck "); // no name — refused
        app.run_command("set grading=sideways"); // not a mode — refused
        app.on_key(key('u')).unwrap();
        assert_eq!(app.status.text, "already at oldest change");
    }

    #[test]
    fn undo_goes_back_to_where_the_change_happened() {
        let mut app = app_with_cards(5);
        app.enter_browse(DECK);
        app.browse_sel = 3;
        app.on_key(key('d')).unwrap();
        app.on_key(key('d')).unwrap();

        app.screen = Screen::Decks;
        app.on_key(key('u')).unwrap();
        assert_eq!(app.screen, Screen::Browse, "back to the browser");
        assert_eq!(app.browse_sel, 3, "and to the row it happened on");
    }

    #[test]
    fn the_undo_history_is_bounded() {
        let mut app = app_with_cards(1);
        app.enter_browse(DECK);
        for _ in 0..UNDO_DEPTH + 10 {
            app.on_key(key('x')).unwrap();
        }
        assert_eq!(app.undo.len(), UNDO_DEPTH, "the oldest steps fall off");
    }

    #[test]
    fn the_review_log_stays_bounded() {
        let mut app = app_with_cards(33);
        app.db.decks[0].new_per_day = 100;
        for _ in 0..MAX_LOG {
            app.db.log.push(ReviewLog {
                date: app.today,
                deck_id: DECK,
                card_id: 1,
                grade: 2,
                drill: false,
                learning: false,
            });
        }
        app.start_session(DECK, Scope::Due);
        app.answer(Grade::Good);
        assert_eq!(app.db.log.len(), MAX_LOG);
        assert_eq!(
            app.db.log.last().map(|e| e.grade),
            Some(Grade::Good.index()),
            "the newest answer is the one kept"
        );
    }

    #[test]
    fn the_clock_rolls_over_past_midnight() {
        let mut app = app_with_cards(2);
        app.today = Local::now().date_naive() - Duration::days(1);
        app.refresh_today();
        assert_eq!(app.today, Local::now().date_naive());
    }

    #[test]
    fn stats_scrolls_instead_of_moving_a_cursor() {
        let mut app = app_with_cards(2);
        app.on_key(key('S')).unwrap();
        assert_eq!(app.screen, Screen::Stats);
        assert_eq!(app.help_scroll, 0);
        app.on_key(key('j')).unwrap();
        assert_eq!(app.help_scroll, 1);
        app.on_key(key('q')).unwrap();
        assert_eq!(app.screen, Screen::Decks, "S remembers where it came from");
    }

    #[test]
    fn a_huge_count_does_not_scroll_backwards() {
        let mut app = app_with_cards(2);
        app.on_key(key('S')).unwrap();
        for ch in "99999".chars() {
            app.on_key(key(ch)).unwrap();
        }
        app.on_key(key('j')).unwrap();
        assert_eq!(app.help_scroll, i16::MAX as u16);
    }

    /// A scratch CSV, named after the test that asked for it.
    fn scratch_csv(name: &str, contents: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("gram-import-{}-{name}.csv", std::process::id()));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn a_failed_import_leaves_no_deck_behind() {
        let mut app = app_with_cards(2);
        let missing = std::env::temp_dir().join("gram-no-such-file.csv");
        let _ = std::fs::remove_file(&missing);

        app.cmd_import(&format!("{} Fresh", missing.display()));

        assert!(app.status.error, "the failure is reported");
        assert_eq!(app.db.decks.len(), 1, "no half-made deck is left over");
        assert!(app.db.deck_index("Fresh").is_none());
        assert!(!app.dirty, "nothing to save");
    }

    #[test]
    fn a_failed_import_does_not_touch_the_target_deck() {
        let mut app = app_with_cards(2);
        let empty = scratch_csv("empty", "");
        let before = app.db.decks[0].cards.clone();

        app.cmd_import(&format!("{} test", empty.display()));

        assert!(app.status.error, "an empty file is an error");
        assert_eq!(app.db.decks[0].cards.len(), before.len());
        assert_eq!(app.db.decks[0].cards[0].term, before[0].term);
        assert!(!app.dirty);
    }

    #[test]
    fn importing_into_a_new_name_creates_the_deck() {
        let mut app = app_with_cards(1);
        // A successful import saves, and `app_with_cards` names its file after
        // the card count — which another test may be holding.
        app.path = std::env::temp_dir().join(format!("gram-import-{}.json", std::process::id()));
        let csv = scratch_csv(
            "fresh",
            "term,definition\nabash,to embarrass\nabate,to subside\n",
        );

        app.cmd_import(&format!("{} Fresh", csv.display()));

        assert!(!app.status.error, "{}", app.status.text);
        let deck = app
            .db
            .deck_by_id(app.db.decks[app.decks_sel].id)
            .expect("the new deck is selected");
        assert_eq!(deck.name, "Fresh");
        assert_eq!(deck.cards.len(), 2);
        assert_ne!(deck.id, 0, "a committed deck gets a real id");
        assert_ne!(deck.id, DECK, "and not the existing deck's");

        let mut ids: Vec<u64> = app
            .db
            .decks
            .iter()
            .flat_map(|d| d.cards.iter().map(|c| c.id))
            .collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "card ids stay unique across decks");
    }

    #[test]
    fn digits_do_not_grade_before_the_answer_is_shown() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(DECK, Scope::Due);

        app.on_key(key('3')).unwrap();
        assert_eq!(app.session.as_ref().unwrap().answered, 0);
        assert!(app.session.as_ref().unwrap().revealed);
    }
}
