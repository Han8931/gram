//! The card editor: opening it, its Vim-style keys, and writing a card back.

use super::*;

impl App {
    pub(super) fn open_new_card(&mut self) {
        let Some(deck) = self.active_deck() else {
            self.error("create a deck first: :deck <name>");
            return;
        };
        self.editor = Some(Editor::blank(deck));
        self.mode = Mode::Insert;
        self.info("new card — Tab/j/k switch fields, ZZ saves, q cancels");
    }

    pub(super) fn open_edit_card(&mut self) {
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

    pub(super) fn edit_review_card(&mut self) {
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
    pub(super) fn commit_editor(&mut self) -> bool {
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
                card.confusing = values[6].clone();
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
                existing.confusing = values[6].clone();
                self.info(format!("saved {}", values[0]));
            }
        }

        self.editor = None;
        self.mode = Mode::Normal;
        self.dirty = true;
        self.refresh_rows();
        true
    }

    pub(super) fn close_editor(&mut self) {
        self.editor = None;
        self.mode = Mode::Normal;
        self.info("cancelled");
    }

    pub(super) fn delete_selected_card(&mut self) {
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

    pub(super) fn key_editor(&mut self, key: KeyEvent) {
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
                KeyCode::Char('c') if ctrl => {
                    self.mode = Mode::Normal;
                    if let Some(editor) = self.editor.as_mut() {
                        editor.current_mut().clamp_normal();
                    }
                }
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

    pub(super) fn field_mut(&mut self, f: impl FnOnce(&mut TextInput)) {
        if let Some(editor) = self.editor.as_mut() {
            f(editor.current_mut());
        }
    }
}
