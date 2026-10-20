//! Study sessions: building the queue, answering, and the review screen's keys.

use super::*;

impl App {
    pub(super) fn build_queue(&mut self, deck_id: u64, scope: Scope) -> VecDeque<u64> {
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
        let extra = match scope {
            Scope::More(extra) => extra,
            _ => 0,
        };
        let allowance = deck.new_allowance(today).saturating_add(extra) as usize;
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
                Scope::Due => format!(
                    "{name}: nothing due — come back tomorrow, or :study more for extra new cards"
                ),
                Scope::More(_) => format!("{name}: nothing due and no unseen cards left"),
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
            Scope::Due | Scope::More(_) => format!("studying {initial} cards — space to reveal"),
            Scope::Flagged(_) => format!(
                "drilling {initial} {} words — space to reveal",
                scope.label()
            ),
        });
    }

    pub(super) fn answer(&mut self, grade: Grade) {
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

        self.carry_answer(deck_id, &snapshot);

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

    pub(super) fn finish_session(&mut self) {
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

    pub(super) fn undo_answer(&mut self) {
        let Some(last) = self.last_answer.take() else {
            self.error("nothing to undo");
            return;
        };
        let card_id = last.card.id;
        let answered = self
            .db
            .deck_by_id(last.deck)
            .and_then(|d| d.card(card_id))
            .cloned();

        if let Some(deck) = self.db.deck_by_id_mut(last.deck) {
            if last.spent_new {
                deck.intro_count = deck.intro_count.saturating_sub(1);
            }
            if let Some(card) = deck.card_mut(card_id) {
                *card = last.card;
            }
        }
        if let Some(answered) = answered {
            self.carry_answer(last.deck, &answered);
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

    /// Turn the card over, from the top of the answer.
    pub(super) fn reveal(&mut self) {
        if let Some(session) = self.session.as_mut() {
            session.revealed = true;
        }
        self.card_scroll = 0;
    }

    /// Review-screen keys. Returns true when the key was consumed.
    pub(super) fn key_review(&mut self, key: KeyEvent) -> bool {
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
                // On the change history, like suspending from the browser,
                // so `u` after the session can take it back.
                if let Some(term) = self
                    .db
                    .deck_by_id(deck_id)
                    .and_then(|d| d.card(card_id))
                    .map(|c| c.term.clone())
                {
                    self.checkpoint(format!("suspending {term}"));
                }
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
}
