//! A single-line text field with Vim-style cursor motions.
//!
//! The cursor counts grapheme clusters — what a reader calls a character — so
//! a letter and the accent on it move, and delete, as one.

use crate::util;

/// Whitespace, as a cluster.
fn is_space(cluster: &str) -> bool {
    cluster.chars().all(char::is_whitespace)
}

#[derive(Debug, Clone, Default)]
pub struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = util::cluster_count(&value);
        Self { value, cursor }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn clusters(&self) -> Vec<&str> {
        util::clusters(&self.value).collect()
    }

    pub fn len(&self) -> usize {
        util::cluster_count(&self.value)
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.value)
    }

    /// Byte offset where cluster `index` starts.
    fn byte_at(&self, index: usize) -> usize {
        util::cluster_offset(&self.value, index)
    }

    // -- editing ---------------------------------------------------------

    pub fn insert(&mut self, ch: char) {
        let at = self.byte_at(self.cursor);
        self.value.insert(at, ch);
        // A combining mark joins the letter before it instead of becoming a
        // cluster of its own, so count the clusters rather than assume one
        // more of them.
        self.cursor = util::cluster_count(&self.value[..at + ch.len_utf8()]);
    }

    #[cfg(test)]
    pub fn insert_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.insert(ch);
        }
    }

    /// Replace everything from character `at` onwards, leaving the cursor at
    /// the end. How Tab completion swaps in a candidate.
    pub fn replace_from(&mut self, at: usize, text: &str) {
        let start = self.byte_at(at);
        self.value.replace_range(start.., text);
        self.cursor = self.len();
    }

    /// Backspace.
    pub fn delete_back(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = self.byte_at(self.cursor - 1);
        let end = self.byte_at(self.cursor);
        self.value.replace_range(start..end, "");
        self.cursor -= 1;
    }

    /// `x` — delete the character under the cursor.
    pub fn delete_under(&mut self) {
        if self.cursor >= self.len() {
            return;
        }
        let start = self.byte_at(self.cursor);
        let end = self.byte_at(self.cursor + 1);
        self.value.replace_range(start..end, "");
    }

    /// `Ctrl-W` — delete the word before the cursor.
    pub fn delete_word_back(&mut self) {
        let parts = self.clusters();
        let mut at = self.cursor;
        while at > 0 && is_space(parts[at - 1]) {
            at -= 1;
        }
        while at > 0 && !is_space(parts[at - 1]) {
            at -= 1;
        }
        let start = self.byte_at(at);
        let end = self.byte_at(self.cursor);
        self.value.replace_range(start..end, "");
        self.cursor = at;
    }

    /// `Ctrl-U` — delete from the line start to the cursor.
    pub fn delete_to_start(&mut self) {
        let end = self.byte_at(self.cursor);
        self.value.replace_range(0..end, "");
        self.cursor = 0;
    }

    /// `D` / `Ctrl-K` — delete from the cursor to the end of the line.
    pub fn delete_to_end(&mut self) {
        let start = self.byte_at(self.cursor);
        self.value.truncate(start);
    }

    // -- motions ---------------------------------------------------------

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// In insert mode the cursor may sit one past the last character.
    pub fn right(&mut self, insert_mode: bool) {
        let max = if insert_mode {
            self.len()
        } else {
            self.len().saturating_sub(1)
        };
        if self.cursor < max {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// `$` in normal mode sits on the last character; `A`/`End` goes past it.
    pub fn end(&mut self, insert_mode: bool) {
        self.cursor = if insert_mode {
            self.len()
        } else {
            self.len().saturating_sub(1)
        };
    }

    /// `w` — start of the next word.
    pub fn word_forward(&mut self) {
        let parts = self.clusters();
        let mut at = self.cursor;
        while at < parts.len() && !is_space(parts[at]) {
            at += 1;
        }
        while at < parts.len() && is_space(parts[at]) {
            at += 1;
        }
        self.cursor = at.min(parts.len());
    }

    /// `b` — start of the previous word.
    pub fn word_back(&mut self) {
        let parts = self.clusters();
        let mut at = self.cursor;
        while at > 0 && is_space(parts[at - 1]) {
            at -= 1;
        }
        while at > 0 && !is_space(parts[at - 1]) {
            at -= 1;
        }
        self.cursor = at;
    }

    /// Clamp the cursor when leaving insert mode, as Vim does.
    pub fn clamp_normal(&mut self) {
        self.cursor = self.cursor.min(self.len().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_and_deletes_multibyte_text() {
        let mut input = TextInput::default();
        input.insert_str("naïve");
        assert_eq!(input.value(), "naïve");
        assert_eq!(input.cursor(), 5);

        input.delete_back();
        assert_eq!(input.value(), "naïv");

        input.home();
        input.delete_under();
        assert_eq!(input.value(), "aïv");
    }

    #[test]
    fn an_accent_belongs_to_the_letter_it_sits_on() {
        let mut input = TextInput::default();
        input.insert('e');
        input.insert('\u{301}'); // combining acute
        assert_eq!(input.value(), "e\u{301}");
        assert_eq!(input.len(), 1, "é is one character to a reader");
        assert_eq!(input.cursor(), 1, "and the cursor is past it, not inside");

        input.delete_back();
        assert_eq!(input.value(), "", "backspace takes the whole letter");
    }

    #[test]
    fn korean_counts_as_one_character_each() {
        let mut input = TextInput::new("한국어");
        assert_eq!(input.len(), 3);
        assert_eq!(input.cursor(), 3);

        input.delete_back();
        assert_eq!(input.value(), "한국");

        input.home();
        input.right(true);
        input.insert('글');
        assert_eq!(input.value(), "한글국");
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn word_motions_step_over_whitespace() {
        let mut input = TextInput::new("to make ashamed");
        input.home();
        input.word_forward();
        assert_eq!(input.cursor(), 3);
        input.word_forward();
        assert_eq!(input.cursor(), 8);
        input.word_back();
        assert_eq!(input.cursor(), 3);
    }

    #[test]
    fn delete_word_back_removes_trailing_space_too() {
        let mut input = TextInput::new("to make ashamed ");
        input.delete_word_back();
        assert_eq!(input.value(), "to make ");
    }

    #[test]
    fn normal_mode_cursor_stops_on_last_char() {
        let mut input = TextInput::new("abc");
        input.end(false);
        assert_eq!(input.cursor(), 2);
        input.end(true);
        assert_eq!(input.cursor(), 3);
    }

    #[test]
    fn clamp_pulls_cursor_back_on_escape() {
        let mut input = TextInput::new("abc");
        assert_eq!(input.cursor(), 3);
        input.clamp_normal();
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn delete_to_start_and_end() {
        let mut input = TextInput::new("hello world");
        input.home();
        input.word_forward();
        input.delete_to_start();
        assert_eq!(input.value(), "world");

        input.home();
        input.right(false);
        input.delete_to_end();
        assert_eq!(input.value(), "w");
    }
}
