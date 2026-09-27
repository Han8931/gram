//! Application state and the Vim-style key dispatcher.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{Duration, Local, NaiveDate};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::model::{Card, CardState, Db, Deck, Grade, Grading, ReviewLog};
use crate::store;
use crate::textinput::TextInput;
use crate::util::Rng;

/// How many cards must pass before a lapsed card comes back around.
const REQUEUE_GAP: usize = 8;

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
    New { deck: usize },
    Existing { deck: usize, card: u64 },
}

pub struct Editor {
    pub target: EditTarget,
    pub fields: Vec<TextInput>,
    pub field: usize,
}

impl Editor {
    fn blank(deck: usize) -> Self {
        Self {
            target: EditTarget::New { deck },
            fields: FIELDS.iter().map(|_| TextInput::default()).collect(),
            field: 0,
        }
    }

    fn from_card(deck: usize, card: &Card) -> Self {
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

    fn deck(&self) -> usize {
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
    /// Only marked cards, regardless of when they are next due.
    Marked,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Due => "due",
            Self::Marked => "marked",
        }
    }
}

pub struct Session {
    pub deck: usize,
    pub scope: Scope,
    pub queue: VecDeque<u64>,
    pub revealed: bool,
    pub answered: u32,
    pub lapses: u32,
    pub initial: usize,
}

impl Session {
    pub fn current(&self) -> Option<u64> {
        self.queue.front().copied()
    }

    pub fn remaining(&self) -> usize {
        self.queue.len()
    }
}

/// Enough state to put a graded card back the way it was.
struct LastAnswer {
    deck: usize,
    card: Card,
    requeued_at: Option<usize>,
    spent_new: bool,
    grade: Grade,
}

struct Deleted {
    deck: usize,
    index: usize,
    card: Card,
}

pub enum ConfirmAction {
    DeleteDeck(usize),
    ForgetDeck(usize),
    DiscardAndQuit,
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

    pub decks_sel: usize,
    pub browse_deck: usize,
    pub browse_sel: usize,
    /// Indices into the browsed deck's cards, after filtering.
    pub rows: Vec<usize>,
    pub filter: String,
    /// `:marked` — show only cards in the study pile.
    pub marked_only: bool,

    pub editor: Option<Editor>,
    pub session: Option<Session>,
    pub confirm: Option<Confirm>,

    /// Backing store for `:` and `/`.
    pub line: TextInput,
    pub status: Status,
    /// A half-typed operator such as `g` or `d`.
    pub pending: Option<char>,
    pub count: String,

    pub help_scroll: u16,
    pub help_from: Screen,

    last_answer: Option<LastAnswer>,
    deleted: Vec<Deleted>,

    pub dirty: bool,
    pub quit: bool,
    rng: Rng,
}

impl App {
    pub fn new(db: Db, path: impl AsRef<Path>) -> Self {
        let mut app = Self {
            db,
            path: path.as_ref().to_path_buf(),
            today: Local::now().date_naive(),
            screen: Screen::Decks,
            mode: Mode::Normal,
            decks_sel: 0,
            browse_deck: 0,
            browse_sel: 0,
            rows: Vec::new(),
            filter: String::new(),
            marked_only: false,
            editor: None,
            session: None,
            confirm: None,
            line: TextInput::default(),
            status: Status::default(),
            pending: None,
            count: String::new(),
            help_scroll: 0,
            help_from: Screen::Decks,
            last_answer: None,
            deleted: Vec::new(),
            dirty: false,
            quit: false,
            rng: Rng::from_entropy(),
        };
        if app.db.decks.is_empty() {
            app.info("no decks yet — press :import <file.csv> or a to add a card");
        }
        app
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

    /// The deck the current screen acts on.
    pub fn active_deck(&self) -> Option<usize> {
        if self.db.decks.is_empty() {
            return None;
        }
        let index = match self.screen {
            Screen::Browse => self.browse_deck,
            Screen::Review => self.session.as_ref().map(|s| s.deck).unwrap_or(0),
            _ => self.decks_sel,
        };
        (index < self.db.decks.len()).then_some(index)
    }

    pub fn selected_card(&self) -> Option<(usize, usize)> {
        let deck = self.browse_deck;
        let index = *self.rows.get(self.browse_sel)?;
        Some((deck, index))
    }

    /// Recompute the filtered row list for the browser.
    pub fn refresh_rows(&mut self) {
        let needle = self.filter.to_lowercase();
        self.rows = match self.db.decks.get(self.browse_deck) {
            None => Vec::new(),
            Some(deck) => deck
                .cards
                .iter()
                .enumerate()
                .filter(|(_, card)| !self.marked_only || card.marked)
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

    fn build_queue(&mut self, deck_index: usize, scope: Scope) -> VecDeque<u64> {
        let Some(deck) = self.db.decks.get(deck_index) else {
            return VecDeque::new();
        };
        let today = self.today;

        // A marked drill ignores due dates: the whole point is to pull those
        // words forward and hammer them.
        if scope == Scope::Marked {
            let mut marked: Vec<u64> = deck
                .cards
                .iter()
                .filter(|c| c.marked && !c.suspended)
                .map(|c| c.id)
                .collect();
            self.rng.shuffle(&mut marked);
            return marked.into();
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

    pub fn start_session(&mut self, deck_index: usize, scope: Scope) {
        let queue = self.build_queue(deck_index, scope);
        if queue.is_empty() {
            let name = self
                .db
                .decks
                .get(deck_index)
                .map(|d| d.name.clone())
                .unwrap_or_default();
            self.info(match scope {
                Scope::Due => format!("{name}: nothing due — come back tomorrow"),
                Scope::Marked => format!("{name}: no marked words — press m to add some"),
            });
            return;
        }
        let initial = queue.len();
        self.session = Some(Session {
            deck: deck_index,
            scope,
            queue,
            revealed: false,
            answered: 0,
            lapses: 0,
            initial,
        });
        self.last_answer = None;
        self.screen = Screen::Review;
        self.info(match scope {
            Scope::Due => format!("studying {initial} cards — space to reveal"),
            Scope::Marked => format!("drilling {initial} marked words — space to reveal"),
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
        let deck_index = session.deck;

        let Some(deck) = self.db.decks.get_mut(deck_index) else {
            return;
        };
        let deck_id = deck.id;
        let Some(card) = deck.card_mut(card_id) else {
            return;
        };

        let snapshot = card.clone();
        let was_new = card.state == CardState::New;
        let requeue = card.grade(grade, today);

        // "I don't know" files the word under things to study. The snapshot
        // above still holds the old flag, so undo clears it again.
        if binary && grade == Grade::Again {
            card.marked = true;
        }

        if was_new {
            deck.spend_new(today, 1);
        }

        self.db.log.push(ReviewLog {
            date: today,
            deck_id,
            card_id,
            grade: grade.index(),
        });

        let session = self.session.as_mut().expect("session checked above");
        session.answered += 1;
        let requeued_at = if requeue {
            session.lapses += 1;
            let at = REQUEUE_GAP.min(session.queue.len());
            session.queue.insert(at, card_id);
            Some(at)
        } else {
            None
        };
        session.revealed = false;

        self.last_answer = Some(LastAnswer {
            deck: deck_index,
            card: snapshot,
            requeued_at,
            spent_new: was_new,
            grade,
        });
        self.dirty = true;

        if self.session.as_ref().is_some_and(|s| s.queue.is_empty()) {
            self.finish_session();
        }
    }

    fn finish_session(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        let name = self
            .db
            .decks
            .get(session.deck)
            .map(|d| d.name.clone())
            .unwrap_or_default();
        self.screen = Screen::Decks;
        self.decks_sel = session.deck;
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

        if let Some(deck) = self.db.decks.get_mut(last.deck) {
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
            }
            session.queue.push_front(card_id);
            session.answered = session.answered.saturating_sub(1);
            session.revealed = true;
        }
        self.dirty = true;
        self.info(format!("undid {}", last.grade.label().to_lowercase()));
    }

    // -- persistence -------------------------------------------------------

    pub fn save(&mut self) -> Result<()> {
        store::save_to(&self.db, &self.path)?;
        self.dirty = false;
        Ok(())
    }

    fn autosave(&mut self) {
        if let Err(err) = self.save() {
            self.error(format!("save failed: {err}"));
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
        let Some((deck_index, card_index)) = self.selected_card() else {
            self.error("no card selected");
            return;
        };
        let card = &self.db.decks[deck_index].cards[card_index];
        self.editor = Some(Editor::from_card(deck_index, card));
        self.mode = Mode::Normal;
        self.info("editing — i to insert, ZZ saves, q cancels");
    }

    fn edit_review_card(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let (deck_index, Some(card_id)) = (session.deck, session.current()) else {
            return;
        };
        let Some(card) = self.db.decks[deck_index].card(card_id) else {
            return;
        };
        self.editor = Some(Editor::from_card(deck_index, card));
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

        let deck_index = editor.deck();
        let target = editor.target;
        let today = self.today;

        match target {
            EditTarget::New { .. } => {
                let id = self.db.next_card_id();
                let Some(deck) = self.db.decks.get_mut(deck_index) else {
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
                    .decks
                    .get_mut(deck_index)
                    .and_then(|d| d.card_mut(card))
                else {
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
        let Some((deck_index, card_index)) = self.selected_card() else {
            self.error("no card selected");
            return;
        };
        let card = self.db.decks[deck_index].cards.remove(card_index);
        let term = card.term.clone();
        self.deleted.push(Deleted {
            deck: deck_index,
            index: card_index,
            card,
        });
        self.dirty = true;
        self.refresh_rows();
        self.info(format!("deleted {term} — u to undo"));
    }

    fn undo_delete(&mut self) {
        let Some(entry) = self.deleted.pop() else {
            self.error("nothing to undo");
            return;
        };
        let term = entry.card.term.clone();
        if let Some(deck) = self.db.decks.get_mut(entry.deck) {
            let at = entry.index.min(deck.cards.len());
            deck.cards.insert(at, entry.card);
        }
        self.dirty = true;
        self.refresh_rows();
        self.info(format!("restored {term}"));
    }

    /// Toggle the study mark on whichever card the screen is showing.
    fn toggle_mark(&mut self) {
        let target = match self.screen {
            Screen::Review => self
                .session
                .as_ref()
                .and_then(|s| s.current().map(|id| (s.deck, id))),
            Screen::Browse => self
                .selected_card()
                .and_then(|(deck, index)| {
                    self.db.decks[deck].cards.get(index).map(|c| (deck, c.id))
                }),
            _ => None,
        };
        let Some((deck_index, card_id)) = target else {
            self.error("no card selected");
            return;
        };
        let Some(card) = self
            .db
            .decks
            .get_mut(deck_index)
            .and_then(|d| d.card_mut(card_id))
        else {
            return;
        };
        card.marked = !card.marked;
        let (term, marked) = (card.term.clone(), card.marked);
        self.dirty = true;
        self.refresh_rows();
        self.info(if marked {
            format!("marked {term} — M drills marked words")
        } else {
            format!("unmarked {term}")
        });
    }

    fn toggle_suspend(&mut self) {
        let Some((deck_index, card_index)) = self.selected_card() else {
            return;
        };
        let card = &mut self.db.decks[deck_index].cards[card_index];
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
        let cursor = self.cursor();
        let next = (*cursor as isize + delta).clamp(0, len as isize - 1);
        *cursor = next as usize;
    }

    fn goto(&mut self, index: usize) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        *self.cursor() = index.min(len - 1);
    }

    /// `n` / `N` — jump to the next row matching the filter.
    fn jump_match(&mut self, forward: bool) {
        if self.filter.is_empty() || self.rows.is_empty() {
            return;
        }
        let len = self.rows.len();
        self.browse_sel = if forward {
            (self.browse_sel + 1) % len
        } else {
            (self.browse_sel + len - 1) % len
        };
    }

    fn enter_browse(&mut self, deck_index: usize) {
        self.browse_deck = deck_index;
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
                Ok(()) => self.quit = true,
                Err(err) => {
                    self.error(format!("save failed: {err} — :q! to quit anyway"));
                }
            }
        } else {
            self.quit = true;
        }
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
            ConfirmAction::DeleteDeck(index) => {
                if index < self.db.decks.len() {
                    let deck = self.db.decks.remove(index);
                    self.decks_sel = self.decks_sel.min(self.db.decks.len().saturating_sub(1));
                    self.dirty = true;
                    self.refresh_rows();
                    self.info(format!("deleted deck {}", deck.name));
                }
            }
            ConfirmAction::ForgetDeck(index) => {
                let today = self.today;
                if let Some(deck) = self.db.decks.get_mut(index) {
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
        }
    }

    /// `:` and `/` share one editable line.
    fn key_line(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.line.clear();
                self.mode = if self.editor.is_some() {
                    Mode::Normal
                } else {
                    Mode::Normal
                };
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
            if key.code == KeyCode::Char('d') {
                self.delete_row();
            }
            return;
        }
        if let Some('Z') = pending {
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

        let count = self.take_count() as isize;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.back(),
            KeyCode::Char('Z') => self.pending = Some('Z'),
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
            KeyCode::Char('g') => self.pending = Some('g'),
            KeyCode::Char('G') => {
                if self.screen == Screen::Help {
                    self.help_scroll = u16::MAX / 2;
                } else if self.count.is_empty() {
                    self.goto(usize::MAX);
                } else {
                    let index = self.take_count().saturating_sub(1);
                    self.goto(index);
                }
            }
            KeyCode::Char('d') => self.pending = Some('d'),

            KeyCode::Char('s') => {
                if let Some(deck) = self.active_deck() {
                    self.start_session(deck, Scope::Due);
                }
            }
            KeyCode::Char('M') => {
                if let Some(deck) = self.active_deck() {
                    self.start_session(deck, Scope::Marked);
                }
            }
            KeyCode::Char('m') if self.screen == Screen::Browse => self.toggle_mark(),
            KeyCode::Char('S') => self.screen = Screen::Stats,
            KeyCode::Char('a') | KeyCode::Char('o') => self.open_new_card(),
            KeyCode::Char('u') => self.undo_delete(),
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
        if self.screen == Screen::Help {
            self.help_scroll = self.help_scroll.saturating_add_signed(delta as i16);
        } else {
            self.move_cursor(delta);
        }
    }

    fn delete_row(&mut self) {
        match self.screen {
            Screen::Browse => self.delete_selected_card(),
            Screen::Decks => {
                let Some(index) = self.active_deck() else {
                    return;
                };
                let deck = &self.db.decks[index];
                self.confirm = Some(Confirm {
                    prompt: format!(
                        "delete deck \"{}\" and its {} cards? [y/N]",
                        deck.name,
                        deck.cards.len()
                    ),
                    action: ConfirmAction::DeleteDeck(index),
                });
            }
            _ => {}
        }
    }

    /// Review-screen keys. Returns true when the key was consumed.
    fn key_review(&mut self, key: KeyEvent) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        let revealed = session.revealed;

        match key.code {
            KeyCode::Char(' ') | KeyCode::Enter if !revealed => {
                if let Some(session) = self.session.as_mut() {
                    session.revealed = true;
                }
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
                if let Some(session) = self.session.as_mut() {
                    session.revealed = true;
                }
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
                if let Some(session) = self.session.as_mut() {
                    session.revealed = true;
                }
                true
            }
            KeyCode::Char('m') => {
                self.toggle_mark();
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
                let deck_index = session.deck;
                if let Some(card) = self
                    .db
                    .decks
                    .get_mut(deck_index)
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
                if let Some(session) = self.session.as_mut() {
                    if let Some(card_id) = session.queue.pop_front() {
                        session.queue.push_back(card_id);
                    }
                }
                self.info("buried until later in the session");
                true
            }
            _ => false,
        }
    }

    fn take_count(&mut self) -> usize {
        if self.count.is_empty() {
            1
        } else {
            let value = self.count.parse().unwrap_or(1);
            self.count.clear();
            value
        }
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
                    Ok(()) => {
                        let path = self.path.display().to_string();
                        self.info(format!("written to {path}"));
                    }
                    Err(err) => self.error(format!("save failed: {err}")),
                }
            }
            "q" | "quit" => {
                if self.editor.is_some() {
                    self.close_editor();
                } else {
                    self.back();
                }
            }
            "q!" | "quit!" => {
                if self.dirty {
                    self.confirm = Some(Confirm {
                        prompt: "discard unsaved changes? [y/N]".into(),
                        action: ConfirmAction::DiscardAndQuit,
                    });
                } else {
                    self.quit = true;
                }
            }
            "wq" | "x" => {
                if self.editor.is_some() && !self.commit_editor() {
                    return;
                }
                self.request_quit();
            }
            "qa" | "qa!" | "quitall" => {
                self.dirty = false;
                self.quit = true;
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
                let index = self.db.add_deck(rest.to_string());
                self.decks_sel = index;
                self.dirty = true;
                self.info(format!("created deck {rest}"));
            }
            "rename" => {
                let Some(index) = self.active_deck() else {
                    self.error("no deck selected");
                    return;
                };
                if rest.is_empty() {
                    self.error("usage: :rename <name>");
                    return;
                }
                self.db.decks[index].name = rest.to_string();
                self.dirty = true;
                self.info(format!("renamed to {rest}"));
            }
            "import" => self.cmd_import(rest),
            "export" => self.cmd_export(rest),
            "study" | "s" => {
                let scope = match rest {
                    "" | "due" => Scope::Due,
                    "marked" | "m" | "star" | "starred" => Scope::Marked,
                    other => {
                        self.error(format!("unknown study scope: {other} (try: marked)"));
                        return;
                    }
                };
                if let Some(index) = self.active_deck() {
                    self.start_session(index, scope);
                }
            }
            "marked" | "mark" => {
                self.marked_only = !self.marked_only;
                if let Some(index) = self.active_deck() {
                    if self.screen != Screen::Browse {
                        self.enter_browse(index);
                    }
                }
                self.browse_sel = 0;
                self.refresh_rows();
                let shown = self.rows.len();
                if self.marked_only {
                    self.info(format!("{shown} marked words — :marked again to show all"));
                } else {
                    self.info("showing all cards");
                }
            }
            "browse" | "b" => {
                if let Some(index) = self.active_deck() {
                    self.enter_browse(index);
                }
            }
            "decks" => self.screen = Screen::Decks,
            "stats" => {
                self.help_from = self.screen;
                self.screen = Screen::Stats;
            }
            "help" | "h" => {
                self.help_from = self.screen;
                self.screen = Screen::Help;
                self.help_scroll = 0;
            }
            "add" | "new" => self.open_new_card(),
            "set" => self.cmd_set(rest),
            "forget" | "reset" => {
                let Some(index) = self.active_deck() else {
                    self.error("no deck selected");
                    return;
                };
                let deck = &self.db.decks[index];
                self.confirm = Some(Confirm {
                    prompt: format!(
                        "reset all scheduling for \"{}\" ({} cards)? [y/N]",
                        deck.name,
                        deck.cards.len()
                    ),
                    action: ConfirmAction::ForgetDeck(index),
                });
            }
            "nohl" | "nohlsearch" | "noh" => {
                self.filter.clear();
                self.marked_only = false;
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
                let Some(index) = self.active_deck() else {
                    self.error("no deck selected");
                    return;
                };
                self.db.decks[index].new_per_day = amount;
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
        } else if let Some(index) = self.active_deck() {
            self.db.decks[index].name.clone()
        } else {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("imported")
                .to_string()
        };

        let index = match self.db.deck_index(&deck_name) {
            Some(index) => index,
            None => self.db.add_deck(deck_name.clone()),
        };

        // `next_card_id` needs `&mut self.db` while the deck is borrowed, so
        // hand the importer a counter it can bump on its own.
        let mut next = self.db.next_card_id();
        let mut deck = std::mem::replace(&mut self.db.decks[index], Deck::new(0, String::new()));
        let result = store::import_csv(
            &mut deck,
            &mut || {
                next += 1;
                next - 1
            },
            &path,
            self.today,
        );
        self.db.decks[index] = deck;

        match result {
            Ok(report) => {
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
                    self.error(format!("imported, but save failed: {err}"));
                }
            }
            Err(err) => self.error(format!("import failed: {err}")),
        }
    }

    fn cmd_export(&mut self, rest: &str) {
        if rest.is_empty() {
            self.error("usage: :export <file.csv>");
            return;
        }
        let Some(index) = self.active_deck() else {
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
        match store::export_csv(&self.db.decks[index], &path) {
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

    /// Share of answers that were not `Again`.
    pub fn retention(&self) -> Option<f32> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Card;

    fn key(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
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
        let path = std::env::temp_dir().join(format!(
            "gram-test-{}-{}.json",
            std::process::id(),
            count
        ));
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
        let queue = app.build_queue(0, Scope::Due);
        assert_eq!(queue.len(), 5);
    }

    #[test]
    fn queue_respects_the_daily_new_limit() {
        let mut app = app_with_cards(50);
        app.db.decks[0].new_per_day = 7;
        assert_eq!(app.build_queue(0, Scope::Due).len(), 7);
    }

    #[test]
    fn suspended_cards_stay_out_of_the_queue() {
        let mut app = app_with_cards(4);
        app.db.decks[0].new_per_day = 100;
        app.db.decks[0].cards[1].suspended = true;
        assert_eq!(app.build_queue(0, Scope::Due).len(), 3);
    }

    #[test]
    fn answering_good_advances_the_queue_and_logs() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(0, Scope::Due);
        assert_eq!(app.session.as_ref().unwrap().remaining(), 3);

        app.answer(Grade::Good);
        assert_eq!(app.session.as_ref().unwrap().remaining(), 2);
        assert_eq!(app.db.log.len(), 1);
        assert_eq!(app.db.decks[0].intro_count, 1);
    }

    #[test]
    fn the_log_records_the_deck_id_not_its_index() {
        let mut app = app_with_cards(2);
        app.db.decks[0].new_per_day = 100;
        app.db.decks[0].id = 77;
        app.start_session(0, Scope::Due);
        app.answer(Grade::Good);
        assert_eq!(app.db.log[0].deck_id, 77);
    }

    #[test]
    fn answering_again_puts_the_card_back() {
        let mut app = app_with_cards(12);
        app.db.decks[0].new_per_day = 100;
        app.start_session(0, Scope::Due);
        let first = app.session.as_ref().unwrap().current().unwrap();

        app.answer(Grade::Again);
        let queue = &app.session.as_ref().unwrap().queue;
        assert_eq!(queue.len(), 12, "card returns to the queue");
        assert!(queue.contains(&first));
        assert_ne!(queue.front(), Some(&first), "but not immediately");
    }

    #[test]
    fn undo_restores_scheduling_and_queue_position() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(0, Scope::Due);
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
        app.start_session(0, Scope::Due);
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
        app.start_session(0, Scope::Due);
        app.answer(Grade::Good);
        assert!(app.session.is_none());
        assert_eq!(app.screen, Screen::Decks);
    }

    #[test]
    fn binary_mode_marks_a_word_you_miss() {
        let mut app = app_with_cards(4);
        app.db.grading = Grading::Binary;
        app.db.decks[0].new_per_day = 100;
        app.start_session(0, Scope::Due);
        let id = app.session.as_ref().unwrap().current().unwrap();

        app.answer(Grade::Again);
        assert!(app.db.decks[0].card(id).unwrap().marked);
    }

    #[test]
    fn sm2_mode_leaves_marks_alone() {
        let mut app = app_with_cards(4);
        app.db.grading = Grading::Sm2;
        app.db.decks[0].new_per_day = 100;
        app.start_session(0, Scope::Due);
        let id = app.session.as_ref().unwrap().current().unwrap();

        app.answer(Grade::Again);
        assert!(!app.db.decks[0].card(id).unwrap().marked);
    }

    #[test]
    fn knowing_a_word_does_not_mark_it() {
        let mut app = app_with_cards(4);
        app.db.grading = Grading::Binary;
        app.db.decks[0].new_per_day = 100;
        app.start_session(0, Scope::Due);
        let id = app.session.as_ref().unwrap().current().unwrap();

        app.answer(Grade::Good);
        assert!(!app.db.decks[0].card(id).unwrap().marked);
    }

    #[test]
    fn undo_clears_a_mark_the_miss_added() {
        let mut app = app_with_cards(4);
        app.db.grading = Grading::Binary;
        app.db.decks[0].new_per_day = 100;
        app.start_session(0, Scope::Due);
        let id = app.session.as_ref().unwrap().current().unwrap();

        app.answer(Grade::Again);
        assert!(app.db.decks[0].card(id).unwrap().marked);

        app.undo_answer();
        assert!(!app.db.decks[0].card(id).unwrap().marked);
    }

    #[test]
    fn a_marked_drill_ignores_due_dates() {
        let mut app = app_with_cards(6);
        // Everything is new and none of it is due, so a normal session with a
        // zero new-card ration would be empty.
        app.db.decks[0].new_per_day = 0;
        assert!(app.build_queue(0, Scope::Due).is_empty());

        app.db.decks[0].cards[1].marked = true;
        app.db.decks[0].cards[4].marked = true;
        let queue = app.build_queue(0, Scope::Marked);
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn a_marked_drill_skips_suspended_cards() {
        let mut app = app_with_cards(4);
        for card in &mut app.db.decks[0].cards {
            card.marked = true;
        }
        app.db.decks[0].cards[0].suspended = true;
        assert_eq!(app.build_queue(0, Scope::Marked).len(), 3);
    }

    #[test]
    fn m_toggles_the_mark_in_the_browser() {
        let mut app = app_with_cards(3);
        app.enter_browse(0);
        app.browse_sel = 2;

        app.on_key(key('m')).unwrap();
        assert!(app.db.decks[0].cards[2].marked);

        app.on_key(key('m')).unwrap();
        assert!(!app.db.decks[0].cards[2].marked);
    }

    #[test]
    fn marked_filter_narrows_the_browser() {
        let mut app = app_with_cards(10);
        app.enter_browse(0);
        app.db.decks[0].cards[3].marked = true;
        app.db.decks[0].cards[7].marked = true;

        app.run_command("marked");
        assert!(app.marked_only);
        assert_eq!(app.rows, vec![3, 7]);

        app.run_command("noh");
        assert!(!app.marked_only);
        assert_eq!(app.rows.len(), 10);
    }

    #[test]
    fn marked_filter_composes_with_search() {
        let mut app = app_with_cards(20);
        app.enter_browse(0);
        for i in [1, 2, 11] {
            app.db.decks[0].cards[i].marked = true;
        }
        app.filter = "term1".to_string();
        app.marked_only = true;
        app.refresh_rows();
        // term1 and term11 match the text; term2 is marked but does not.
        assert_eq!(app.rows, vec![1, 11]);
    }

    #[test]
    fn study_marked_starts_a_marked_session() {
        let mut app = app_with_cards(5);
        app.db.decks[0].cards[0].marked = true;
        app.run_command("study marked");

        let session = app.session.as_ref().expect("session started");
        assert_eq!(session.scope, Scope::Marked);
        assert_eq!(session.remaining(), 1);
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
        assert_eq!(app.db.grading, Grading::Binary, "a bad value changes nothing");
    }

    #[test]
    fn y_and_n_grade_only_after_the_reveal() {
        let mut app = app_with_cards(4);
        app.db.grading = Grading::Binary;
        app.db.decks[0].new_per_day = 100;
        app.start_session(0, Scope::Due);

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
        app.start_session(0, Scope::Due);

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
        app.enter_browse(0);
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
        app.enter_browse(0);
        app.on_key(key('5')).unwrap();
        app.on_key(key('j')).unwrap();
        assert_eq!(app.browse_sel, 5);
    }

    #[test]
    fn gg_and_G_jump_to_the_ends() {
        let mut app = app_with_cards(20);
        app.enter_browse(0);
        app.on_key(key('G')).unwrap();
        assert_eq!(app.browse_sel, 19);

        app.on_key(key('g')).unwrap();
        app.on_key(key('g')).unwrap();
        assert_eq!(app.browse_sel, 0);
    }

    #[test]
    fn search_filters_rows() {
        let mut app = app_with_cards(15);
        app.enter_browse(0);
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
        app.enter_browse(0);
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
        app.enter_browse(0);
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
        app.start_session(0, Scope::Due);

        app.on_key(key(' ')).unwrap();
        assert!(app.session.as_ref().unwrap().revealed);

        app.on_key(key(' ')).unwrap();
        assert!(!app.session.as_ref().unwrap().revealed);
        assert_eq!(app.session.as_ref().unwrap().answered, 1);
    }

    #[test]
    fn digits_do_not_grade_before_the_answer_is_shown() {
        let mut app = app_with_cards(3);
        app.db.decks[0].new_per_day = 100;
        app.start_session(0, Scope::Due);

        app.on_key(key('3')).unwrap();
        assert_eq!(app.session.as_ref().unwrap().answered, 0);
        assert!(app.session.as_ref().unwrap().revealed);
    }
}
