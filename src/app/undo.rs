//! The change history behind `u` and `Ctrl-r`.

use super::*;

impl App {
    /// Record the collection as it stands, ahead of the change `label`
    /// describes. Every path that sets `dirty` calls this first, and calls it
    /// after its own validation, so a refused change leaves no empty step for
    /// `u` to walk through.
    pub(super) fn checkpoint(&mut self, label: impl Into<String>) {
        // A new change makes the redo stack unreachable, as in Vim.
        self.redo.clear();
        if self.undo.len() == UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.undo.push(self.step(label.into()));
    }

    /// The here and now, ready to be pushed onto either stack.
    pub(super) fn step(&self, label: String) -> Step {
        Step {
            db: self.db.without_log(),
            label,
            screen: self.screen,
            deck: self.browse_deck,
            row: self.browse_sel,
        }
    }

    /// `u` — take back the last `count` changes.
    pub(super) fn undo(&mut self, count: usize) {
        self.walk(count, true);
    }

    /// `Ctrl-r` — put back the last `count` changes that `u` took away.
    pub(super) fn redo(&mut self, count: usize) {
        self.walk(count, false);
    }

    pub(super) fn walk(&mut self, count: usize, back: bool) {
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
    pub(super) fn restore_step(&mut self, step: Step) {
        // The log is the one part a snapshot leaves out: it records answers,
        // which undo never takes back.
        let log = std::mem::take(&mut self.db.log);
        self.db = step.db;
        self.db.log = log;
        self.browse_deck = step.deck;
        self.browse_sel = step.row;

        // The deck the step was taken in may not exist in the state being
        // restored — undoing `:deck foo` is exactly that case — and a browser
        // pointed at a deck that is not there shows "no deck" and traps you.
        let deck_gone = self.db.deck_by_id(self.browse_deck).is_none();
        self.screen = match step.screen {
            Screen::Browse | Screen::Card if deck_gone => Screen::Decks,
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

    /// Write an answer — or the taking back of one — into every snapshot.
    /// Answers are not on the change history, so `u` must never undo them,
    /// but each snapshot holds the whole collection: without this, undoing a
    /// status set before a session would quietly take the session back with
    /// it. `before` is the card as it was ahead of the answer; the status is
    /// carried only if the answer itself changed it, so a status you set by
    /// hand stays undoable.
    pub(super) fn carry_answer(&mut self, deck_id: u64, before: &Card) {
        let Some(deck) = self.db.deck_by_id(deck_id) else {
            return;
        };
        let Some(after) = deck.card(before.id).cloned() else {
            return;
        };
        let (intro_date, intro_count) = (deck.intro_date, deck.intro_count);
        let status_changed = before.status != after.status;
        for step in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            let Some(deck) = step.db.deck_by_id_mut(deck_id) else {
                continue;
            };
            deck.intro_date = intro_date;
            deck.intro_count = intro_count;
            if let Some(card) = deck.card_mut(after.id) {
                card.copy_schedule(&after);
                if status_changed {
                    card.status = after.status;
                }
            }
        }
    }

    pub(super) fn clamp_decks_sel(&mut self) {
        if self.decks_sel >= self.db.decks.len() {
            self.decks_sel = self.db.decks.len().saturating_sub(1);
        }
    }
}
