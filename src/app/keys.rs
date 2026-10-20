//! Key dispatch, and the moves between screens it drives.

use super::*;

impl App {
    pub(super) fn back(&mut self) {
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
                self.search.clear();
                self.search_at = None;
            }
            Screen::Card => {
                self.screen = Screen::Browse;
                self.card_scroll = 0;
            }
            Screen::Decks => self.request_quit(),
        }
    }

    pub(super) fn request_quit(&mut self) {
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
    pub(super) fn request_discard_quit(&mut self) {
        if self.dirty {
            self.confirm = Some(Confirm {
                prompt: "discard unsaved changes? [y/N]".into(),
                action: ConfirmAction::DiscardAndQuit,
            });
        } else {
            self.quit = true;
        }
    }

    pub(super) fn open_stats(&mut self) {
        if self.screen != Screen::Stats {
            self.help_from = self.screen;
        }
        self.screen = Screen::Stats;
        self.help_scroll = 0;
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        if self.confirm.is_some() {
            self.key_confirm(key);
            return Ok(());
        }
        match self.mode {
            Mode::Command | Mode::Search | Mode::Filter => self.key_line(key),
            _ if self.editor.is_some() => self.key_editor(key),
            _ => self.key_screen(key),
        }
        Ok(())
    }

    pub(super) fn key_confirm(&mut self, key: KeyEvent) {
        let answer = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => true,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => false,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => false,
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

    /// `:` and `/` share one editable line.
    pub(super) fn key_line(&mut self, key: KeyEvent) {
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

        let before = self.line.value().len();
        match key.code {
            // Ctrl-C leaves the line the way Esc does, as in Vim.
            KeyCode::Esc => self.leave_line(),
            KeyCode::Char('c') if ctrl => self.leave_line(),
            KeyCode::Enter => {
                let text = self.line.take();
                let mode = self.mode;
                self.mode = Mode::Normal;
                self.line_origin = None;
                match mode {
                    Mode::Search => self.finish_search(text),
                    Mode::Filter => {
                        self.filter = text;
                        self.refresh_rows();
                        let hits = self.rows.len();
                        if self.filter.is_empty() {
                            self.info("filter cleared");
                        } else if hits == 0 {
                            self.error(format!("no cards match {:?}", self.filter));
                        } else {
                            self.info(format!("{hits} matching — :noh to show all"));
                        }
                    }
                    _ => self.run_command(&text),
                }
            }
            KeyCode::Backspace => {
                if self.line.is_empty() {
                    self.leave_line();
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
        if self.line.value().len() != before {
            self.follow_line();
        }
    }

    /// Tab on the command line. The first press fills in as far as the
    /// candidates agree; once there is nothing left to fill, further presses
    /// cycle through them, and `BackTab` cycles the other way.
    pub(super) fn complete_line(&mut self, forward: bool) {
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

    pub(super) fn key_screen(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Raw mode turns Ctrl-C into a key like any other. Say what quits
        // instead, the way Vim does, rather than swallowing it in silence.
        if ctrl && key.code == KeyCode::Char('c') {
            self.pending = None;
            self.count.clear();
            self.info("type :q to quit, or ZZ to save and quit");
            return;
        }

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
                KeyCode::Char('g') if self.screen == Screen::Card => {
                    self.count.clear();
                    self.card_scroll = 0;
                }
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
        if self.screen == Screen::Card && self.key_card(key) {
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
            // Esc puts a search highlight away before it leaves the screen.
            KeyCode::Esc if self.screen == Screen::Browse && self.hide_search() => {}
            KeyCode::Char('q') | KeyCode::Esc => self.back(),
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.line.clear();
            }
            KeyCode::Char('/') if self.screen == Screen::Browse => self.open_line(Mode::Search),
            KeyCode::Char('f') if self.screen == Screen::Browse && !ctrl => {
                self.open_line(Mode::Filter)
            }
            KeyCode::Char('?') => {
                if self.screen != Screen::Help {
                    self.help_from = self.screen;
                }
                self.screen = Screen::Help;
                self.help_scroll = 0;
            }
            KeyCode::Char('n') if matches!(self.screen, Screen::Browse | Screen::Card) => {
                self.jump_match(true)
            }
            KeyCode::Char('N') if matches!(self.screen, Screen::Browse | Screen::Card) => {
                self.jump_match(false)
            }

            KeyCode::Char('j') | KeyCode::Down => self.scroll(count),
            KeyCode::Char('k') | KeyCode::Up => self.scroll(-count),
            // Shift-] and Shift-[ — ten rows a press, for a long list where
            // j and k are too slow and Ctrl-d is a stretch. `3}` goes thirty.
            KeyCode::Char('}') => self.scroll(10 * count),
            KeyCode::Char('{') => self.scroll(-10 * count),
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
            KeyCode::Char('p')
                if matches!(self.screen, Screen::Browse | Screen::Card | Screen::Review) =>
            {
                self.pronounce()
            }
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
                Screen::Browse => self.open_card_view(),
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

    pub(super) fn scroll(&mut self, delta: isize) {
        // Clamp before narrowing: a big count such as `99999j` would
        // otherwise wrap round to a negative i16 and scroll backwards.
        let narrow = |delta: isize| delta.clamp(i16::MIN as isize, i16::MAX as isize) as i16;
        match self.screen {
            Screen::Help | Screen::Stats => {
                self.help_scroll = self.help_scroll.saturating_add_signed(narrow(delta));
            }
            // A long answer scrolls inside the card. The render clamps it to
            // whatever actually overflows.
            Screen::Review | Screen::Card => {
                self.card_scroll = self.card_scroll.saturating_add_signed(narrow(delta));
            }
            _ => self.move_cursor(delta),
        }
    }

    pub(super) fn delete_row(&mut self) {
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

    /// Consume the pending count, if the user typed one.
    pub(super) fn take_count_opt(&mut self) -> Option<usize> {
        if self.count.is_empty() {
            return None;
        }
        let value = self.count.parse().ok();
        self.count.clear();
        value
    }

    pub(super) fn take_count(&mut self) -> usize {
        self.take_count_opt().unwrap_or(1)
    }
}
