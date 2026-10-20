//! The card browser and the word view: moving about, statuses, suspending.

use super::*;

impl App {
    /// The card the current screen is pointing at, as `(deck id, card id)`.
    pub(super) fn current_card(&self) -> Option<(u64, u64)> {
        match self.screen {
            Screen::Review => self
                .session
                .as_ref()
                .and_then(|s| s.current().map(|id| (s.deck, id))),
            Screen::Browse | Screen::Card => self.selected_card().and_then(|(deck, index)| {
                let card = self.db.deck_by_id(deck)?.cards.get(index)?;
                Some((deck, card.id))
            }),
            _ => None,
        }
    }

    /// `m` — step this word to the next status; `M` is the drill, so the
    /// reverse step lives on `Ctrl-m`.
    pub(super) fn cycle_status(&mut self, forward: bool) {
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
    pub(super) fn set_status(&mut self, status: Study) {
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

    pub(super) fn toggle_suspend(&mut self) {
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

    pub(super) fn list_len(&self) -> usize {
        match self.screen {
            Screen::Decks => self.db.decks.len(),
            Screen::Browse => self.rows.len(),
            _ => 0,
        }
    }

    pub(super) fn cursor(&mut self) -> &mut usize {
        match self.screen {
            Screen::Browse => &mut self.browse_sel,
            _ => &mut self.decks_sel,
        }
    }

    pub(super) fn move_cursor(&mut self, delta: isize) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        self.peek = false;
        let cursor = self.cursor();
        let next = (*cursor as isize + delta).clamp(0, len as isize - 1);
        *cursor = next as usize;
    }

    pub(super) fn goto(&mut self, index: usize) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        self.peek = false;
        *self.cursor() = index.min(len - 1);
    }

    /// Enter on the browser — open the word to read it, not to edit it.
    pub(super) fn open_card_view(&mut self) {
        if self.selected_card().is_none() {
            self.error("no card selected");
            return;
        }
        self.screen = Screen::Card;
        self.card_scroll = 0;
        self.peek = false;
    }

    /// `J` / `K` in the word view: the next word down the list, or up it.
    pub(super) fn step_card(&mut self, delta: isize) {
        let len = self.rows.len();
        if len == 0 {
            return;
        }
        let next = (self.browse_sel as isize + delta).clamp(0, len as isize - 1) as usize;
        if next == self.browse_sel {
            self.info(if delta > 0 { "last word" } else { "first word" });
            return;
        }
        self.browse_sel = next;
        self.card_scroll = 0;
    }

    /// `z` — cover the meanings, or uncover them if they are already covered.
    /// The whole list at once: it is the list you are testing yourself against.
    pub(super) fn toggle_hide_meanings(&mut self) {
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
    pub(super) fn toggle_peek(&mut self) {
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

    pub(super) fn enter_browse(&mut self, deck_id: u64) {
        self.browse_deck = deck_id;
        self.browse_sel = 0;
        self.screen = Screen::Browse;
        self.refresh_rows();
    }

    /// Keys of the word view. Returns true when the key was consumed; `j` /
    /// `k`, `n` / `N` and `q` fall through to the shared bindings.
    pub(super) fn key_card(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('J') | KeyCode::Tab => self.step_card(1),
            KeyCode::Char('K') | KeyCode::BackTab => self.step_card(-1),
            KeyCode::Char('i') | KeyCode::Char('e') | KeyCode::Enter => self.open_edit_card(),
            KeyCode::Char('m') => self.cycle_status(!ctrl),
            KeyCode::Char('x') if !ctrl => self.toggle_suspend(),
            // `gg` and `G` go to the top and bottom of the card, not the deck.
            KeyCode::Char('G') => self.card_scroll = u16::MAX / 2,
            _ => return false,
        }
        true
    }
}
