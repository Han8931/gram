//! `:` commands, and `:restore` with them.

use super::*;

impl App {
    /// `:restore` — put a kept copy back, or say which copies there are.
    pub(super) fn begin_restore(&mut self, rest: &str) {
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
    pub(super) fn restore(&mut self, backup: &Path) {
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

    pub(super) fn run_command(&mut self, raw: &str) {
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
                let (name, count) = match rest.split_once(char::is_whitespace) {
                    Some((name, count)) => (name, Some(count.trim())),
                    None => (rest, None),
                };
                if count.is_some() && !matches!(name, "more" | "extra") {
                    self.error(format!(
                        "only :study more takes a number, not :study {name}"
                    ));
                    return;
                }
                let scope = match name {
                    "" | "due" => Scope::Due,
                    // `:study more` adds another day's ration; `:study more 30`
                    // says how many.
                    "more" | "extra" => {
                        let extra = match count {
                            Some(count) => match count.parse::<u32>() {
                                Ok(extra) if extra > 0 => extra,
                                _ => {
                                    self.error(format!("not a positive number: {count}"));
                                    return;
                                }
                            },
                            None => self
                                .active_deck()
                                .and_then(|id| self.db.deck_by_id(id))
                                .map_or(0, |deck| deck.new_per_day)
                                .max(10),
                        };
                        Scope::More(extra)
                    }
                    "flagged" | "all" => Scope::Flagged(None),
                    other => match Study::parse(other) {
                        Some(Study::None) => Scope::Flagged(None),
                        Some(status) => Scope::Flagged(Some(status)),
                        None => {
                            self.error(format!(
                                "unknown study scope: {other} (try: due, more, flagged, review, marked, test)"
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
                self.search.clear();
                self.search_at = None;
                self.status_filter = StatusFilter::Off;
                self.refresh_rows();
            }
            other => self.error(format!("unknown command: {other}")),
        }
    }

    pub(super) fn cmd_set(&mut self, rest: &str) {
        let Some((key, value)) = rest.split_once('=') else {
            self.error("usage: :set new=<n> | grading=binary|sm2 | reverse=on|off | voice=<name>");
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
            "reverse" => {
                let reverse = match value {
                    "on" | "yes" | "true" | "1" => true,
                    "off" | "no" | "false" | "0" => false,
                    _ => {
                        self.error(format!("reverse must be on or off, not {value:?}"));
                        return;
                    }
                };
                let Some(deck_id) = self.active_deck() else {
                    self.error("no deck selected");
                    return;
                };
                self.checkpoint("the card direction");
                if let Some(deck) = self.db.deck_by_id_mut(deck_id) {
                    deck.reverse = reverse;
                }
                self.dirty = true;
                self.info(if reverse {
                    "reverse — recall the word from its meaning"
                } else {
                    "forward — recall the meaning from the word"
                });
            }
            // Whatever the speech program calls it: `Tingting` for `say`,
            // `cmn` for espeak. Nothing after the `=` goes back to picking
            // one from the deck's script.
            "voice" => {
                let Some(deck_id) = self.active_deck() else {
                    self.error("no deck selected");
                    return;
                };
                self.checkpoint("the voice");
                if let Some(deck) = self.db.deck_by_id_mut(deck_id) {
                    deck.voice = value.to_string();
                }
                self.dirty = true;
                self.info(if value.is_empty() {
                    "voice chosen from the deck's script — p to hear it".to_string()
                } else {
                    format!("voice {value} — p to hear it")
                });
            }
            other => self.error(format!("unknown setting: {other}")),
        }
    }

    pub(super) fn cmd_import(&mut self, rest: &str) {
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

    pub(super) fn cmd_export(&mut self, rest: &str) {
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
}
