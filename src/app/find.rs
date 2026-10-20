//! `/` and `f`: finding a word in the browser as you type it.

use super::*;

impl App {
    /// What the browser is searching for: the line being typed while `/` is
    /// open, the last search after it.
    pub fn search_query(&self) -> &str {
        if self.mode == Mode::Search {
            self.line.value()
        } else {
            &self.search
        }
    }

    /// Whether the browser should pick out the matches: always while `/` is
    /// being typed, and afterwards only until the cursor leaves the match it
    /// was taken to.
    pub fn search_highlighted(&self) -> bool {
        self.mode == Mode::Search
            || self
                .search_at
                .is_some_and(|card| self.rows.get(self.browse_sel) == Some(&card))
    }

    /// Put the highlight away without forgetting the search, so `n` and `N`
    /// still work. Returns whether there was one to put away.
    pub(super) fn hide_search(&mut self) -> bool {
        let shown = self.search_highlighted();
        self.search_at = None;
        shown
    }

    /// Rows matching the search, as positions in the list, best match first.
    pub fn search_matches(&self) -> Vec<usize> {
        let query = Query::new(self.search_query());
        if query.is_empty() {
            return Vec::new();
        }
        let Some(deck) = self.db.deck_by_id(self.browse_deck) else {
            return Vec::new();
        };
        search::rank(
            self.rows
                .iter()
                .enumerate()
                .filter_map(|(row, &i)| Some((query.score(deck.cards.get(i)?)?, row)))
                .collect(),
        )
    }

    /// `n` / `N` — on to the next match of the last search, best first, the
    /// way Vim steps through a pattern. With no search, but a filter, every
    /// row is a match and they are stepped through in order.
    pub(super) fn jump_match(&mut self, forward: bool) {
        if self.rows.is_empty() {
            return;
        }
        self.peek = false;
        self.card_scroll = 0;
        if self.search.is_empty() {
            if self.filter.is_empty() {
                self.error("no previous search — / to search");
                return;
            }
            let len = self.rows.len();
            self.browse_sel = if forward {
                (self.browse_sel + 1) % len
            } else {
                (self.browse_sel + len - 1) % len
            };
            return;
        }
        let matches = self.search_matches();
        if matches.is_empty() {
            self.error(format!("no match for {:?}", self.search));
            return;
        }
        let len = matches.len();
        let next = match matches.iter().position(|&row| row == self.browse_sel) {
            Some(at) if forward => (at + 1) % len,
            Some(at) => (at + len - 1) % len,
            None if forward => 0,
            None => len - 1,
        };
        self.browse_sel = matches[next];
        self.search_at = self.rows.get(self.browse_sel).copied();
        self.info(format!("match {} of {len} for {:?}", next + 1, self.search));
    }

    /// `/` and `f` both open the line on the browser. Remember how things
    /// stood, so `Esc` can leave them that way.
    pub(super) fn open_line(&mut self, mode: Mode) {
        self.line_origin = Some((self.browse_sel, self.filter.clone()));
        self.line.clear();
        self.mode = mode;
    }

    /// Re-run a half-typed `/` or `f` after every key, so the list answers as
    /// you type instead of after Enter.
    pub(super) fn follow_line(&mut self) {
        match self.mode {
            Mode::Search => {
                let origin = self.line_origin.as_ref().map_or(0, |(row, _)| *row);
                self.peek = false;
                self.browse_sel = self.search_matches().first().copied().unwrap_or(origin);
            }
            Mode::Filter => {
                self.filter = self.line.value().to_string();
                self.browse_sel = 0;
                self.peek = false;
                self.refresh_rows();
            }
            _ => {}
        }
    }

    /// `Esc` out of `/` or `f`: the cursor and the filter go back to where
    /// they were before it opened.
    pub(super) fn cancel_line(&mut self) {
        if let Some((row, filter)) = self.line_origin.take()
            && matches!(self.mode, Mode::Search | Mode::Filter)
        {
            if self.filter != filter {
                self.filter = filter;
                self.refresh_rows();
            }
            self.browse_sel = row.min(self.rows.len().saturating_sub(1));
        }
    }

    pub(super) fn leave_line(&mut self) {
        self.cancel_line();
        self.line.clear();
        // The editor is tracked separately, so leaving the command line
        // always just means "back to normal mode".
        self.mode = Mode::Normal;
    }

    /// Enter on `/`: the cursor stays on the best match, and `n` / `N` take
    /// over from there.
    pub(super) fn finish_search(&mut self, text: String) {
        if text.trim().is_empty() {
            // Vim's bare `/` repeats the last search.
            self.jump_match(true);
            return;
        }
        self.search = text;
        let matches = self.search_matches();
        match matches.first() {
            None => self.error(format!("no match for {:?}", self.search)),
            Some(&best) => {
                self.browse_sel = best;
                self.search_at = self.rows.get(best).copied();
                let rows = &self.rows;
                let fuzzy = Query::new(&self.search);
                let close = self
                    .db
                    .deck_by_id(self.browse_deck)
                    .and_then(|deck| deck.cards.get(rows[best]))
                    .and_then(|card| fuzzy.score(card))
                    .is_some_and(|score| score.is_fuzzy());
                let total = matches.len();
                self.info(if close {
                    format!("no exact match — {total} close spellings, n / N to step through")
                } else if total == 1 {
                    "1 match".to_string()
                } else {
                    format!("{total} matches, best first — n / N to step through")
                });
            }
        }
    }
}
