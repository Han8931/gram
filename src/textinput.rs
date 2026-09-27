//! A single-line text field with Vim-style cursor motions.
//!
//! The cursor is a *character* index, so multi-byte input behaves sanely.

#[derive(Debug, Clone, Default)]
pub struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        Self { value, cursor }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn len(&self) -> usize {
        self.value.chars().count()
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

    /// Byte offset of character index `index`.
    fn byte_at(&self, index: usize) -> usize {
        self.value
            .char_indices()
            .nth(index)
            .map(|(offset, _)| offset)
            .unwrap_or(self.value.len())
    }

    // -- editing ---------------------------------------------------------

    pub fn insert(&mut self, ch: char) {
        let at = self.byte_at(self.cursor);
        self.value.insert(at, ch);
        self.cursor += 1;
    }

    #[cfg(test)]
    pub fn insert_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.insert(ch);
        }
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
        let chars: Vec<char> = self.value.chars().collect();
        let mut at = self.cursor;
        while at > 0 && chars[at - 1].is_whitespace() {
            at -= 1;
        }
        while at > 0 && !chars[at - 1].is_whitespace() {
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
        let chars: Vec<char> = self.value.chars().collect();
        let mut at = self.cursor;
        while at < chars.len() && !chars[at].is_whitespace() {
            at += 1;
        }
        while at < chars.len() && chars[at].is_whitespace() {
            at += 1;
        }
        self.cursor = at.min(chars.len());
    }

    /// `b` — start of the previous word.
    pub fn word_back(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let mut at = self.cursor;
        while at > 0 && chars[at - 1].is_whitespace() {
            at -= 1;
        }
        while at > 0 && !chars[at - 1].is_whitespace() {
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
