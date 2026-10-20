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
            db: self.db.clone(),
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
        self.db = step.db;
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

    pub(super) fn clamp_decks_sel(&mut self) {
        if self.decks_sel >= self.db.decks.len() {
            self.decks_sel = self.db.decks.len().saturating_sub(1);
        }
    }
}
