//! A small multi-line text input. ratatui has none built in.

use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Default)]
pub struct Input {
    text: String,
    /// Byte offset into `text`, always on a char boundary.
    cursor: usize,
}

impl Input {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn insert(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    /// Applies an editing key. Returns `false` for keys it does not handle.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Enter if alt || key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.insert("\n");
            }
            KeyCode::Char('j') if ctrl => self.insert("\n"),
            // Emacs keys, as in Claude Code and the shell.
            KeyCode::Char('a') if ctrl => self.cursor = self.line_start(),
            KeyCode::Char('e') if ctrl => self.cursor = self.line_end(),
            KeyCode::Char('u') if ctrl => self.delete_to(self.line_start()),
            KeyCode::Char('k') if ctrl => self.delete_to(self.line_end()),
            KeyCode::Char('w') if ctrl => self.delete_to(self.word_start()),
            KeyCode::Backspace if alt => self.delete_to(self.word_start()),
            KeyCode::Char('b') if alt => self.cursor = self.word_start(),
            KeyCode::Char('f') if alt => self.cursor = self.word_end(),
            KeyCode::Left if ctrl || alt => self.cursor = self.word_start(),
            KeyCode::Right if ctrl || alt => self.cursor = self.word_end(),
            KeyCode::Char(c) if !ctrl && !alt => self.insert(c.encode_utf8(&mut [0; 4])),
            KeyCode::Backspace => {
                if let Some(prev) = self.prev_boundary() {
                    self.delete_to(prev);
                }
            }
            KeyCode::Delete => {
                if let Some(next) = self.next_boundary() {
                    self.delete_to(next);
                }
            }
            KeyCode::Left => self.cursor = self.prev_boundary().unwrap_or(self.cursor),
            KeyCode::Right => self.cursor = self.next_boundary().unwrap_or(self.cursor),
            KeyCode::Home => self.cursor = self.line_start(),
            KeyCode::End => self.cursor = self.line_end(),
            _ => return false,
        }
        true
    }

    /// Splits the text into rows of at most `width` characters, and finds the cursor as (column, row).
    pub fn layout(&self, width: u16) -> (Vec<String>, (u16, u16)) {
        let (rows, cursor) = self.rows(width);
        let rows = rows.into_iter().map(|r| self.text[r].to_owned()).collect();
        let to_u16 = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
        (rows, (to_u16(cursor.0), to_u16(cursor.1)))
    }

    /// The rows of at most `width` characters, as byte ranges of the text, and the cursor as
    /// (column, row). Rows break after the last space that fits, or inside a word that doesn't.
    fn rows(&self, width: u16) -> (Vec<Range<usize>>, (usize, usize)) {
        let width = usize::from(width.max(1));
        let mut rows = Vec::new();
        let mut cursor = (0, 0);
        let mut start = 0;
        for line in self.text.split('\n') {
            let end = start + line.len();
            let first = rows.len();
            let mut row_start = start;
            let mut chars = 0;
            // Where the row can break after its last space, and its characters up to there.
            let mut space = None;
            for (i, c) in line.char_indices() {
                if chars == width {
                    let (at, count) = space.take().unwrap_or((start + i, chars));
                    rows.push(row_start..at);
                    row_start = at;
                    chars -= count;
                }
                chars += 1;
                if c == ' ' {
                    space = Some((start + i + 1, chars));
                }
            }
            rows.push(row_start..end);
            if (start..=end).contains(&self.cursor) {
                let row = (first..rows.len())
                    .rfind(|&r| rows[r].start <= self.cursor)
                    .unwrap_or(first);
                let column = self.text[rows[row].start..self.cursor].chars().count();
                cursor = (column, row);
                // The cursor sits right after a full row: give it an empty row to be on.
                if column == width {
                    cursor = (0, row + 1);
                    rows.push(end..end);
                }
            }
            start = end + 1;
        }
        (rows, cursor)
    }

    /// Deletes between the cursor and `to`, on either side of it.
    fn delete_to(&mut self, to: usize) {
        let range = self.cursor.min(to)..self.cursor.max(to);
        self.cursor = range.start;
        self.text.replace_range(range, "");
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |i| self.cursor + i)
    }

    /// The start of the word before the cursor, skipping what is between.
    fn word_start(&self) -> usize {
        self.text[..self.cursor]
            .trim_end_matches(|c: char| !c.is_alphanumeric())
            .trim_end_matches(char::is_alphanumeric)
            .len()
    }

    /// The end of the word after the cursor, skipping what is between.
    fn word_end(&self) -> usize {
        let rest = self.text[self.cursor..]
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .trim_start_matches(char::is_alphanumeric);
        self.text.len() - rest.len()
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
    }

    fn next_boundary(&self) -> Option<usize> {
        let c = self.text[self.cursor..].chars().next()?;
        Some(self.cursor + c.len_utf8())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Input {
        fn new(text: String) -> Self {
            Self {
                cursor: text.len(),
                text,
            }
        }
    }

    fn press(input: &mut Input, code: KeyCode, modifiers: KeyModifiers) {
        assert!(input.handle_key(KeyEvent::new(code, modifiers)));
    }

    #[test]
    fn edits_around_multibyte_chars() {
        let mut input = Input::new("héllo".into());
        press(&mut input, KeyCode::Left, KeyModifiers::NONE);
        press(&mut input, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(input.text(), "hélo");
        press(&mut input, KeyCode::Home, KeyModifiers::NONE);
        press(&mut input, KeyCode::Right, KeyModifiers::NONE);
        press(&mut input, KeyCode::Delete, KeyModifiers::NONE);
        assert_eq!(input.text(), "hlo");
        press(&mut input, KeyCode::Char('j'), KeyModifiers::CONTROL);
        press(&mut input, KeyCode::Char('ü'), KeyModifiers::NONE);
        assert_eq!(input.text(), "h\nülo");
        press(&mut input, KeyCode::End, KeyModifiers::NONE);
        press(&mut input, KeyCode::Enter, KeyModifiers::ALT);
        assert_eq!(input.text(), "h\nülo\n");
    }

    #[test]
    fn plain_enter_is_left_to_the_caller() {
        let mut input = Input::default();
        assert!(!input.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        assert!(!input.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn layout_wraps_rows_and_places_the_cursor() {
        let input = Input::new("abcdef\nxy".into());
        assert_eq!(
            input.layout(4),
            (vec!["abcd".into(), "ef".into(), "xy".into()], (2, 2))
        );

        let mut input = Input::new("abcd".into());
        assert_eq!(
            input.layout(4),
            (vec!["abcd".into(), String::new()], (0, 1))
        );
        input.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        assert_eq!(input.layout(4), (vec!["abcd".into()], (0, 0)));

        assert_eq!(Input::default().layout(4), (vec![String::new()], (0, 0)));
        assert_eq!(
            Input::new("a\n".into()).layout(4),
            (vec!["a".into(), String::new()], (0, 1))
        );

        // After a full row that isn't the last, too.
        let mut input = Input::new("abcd\nxy".into());
        input.cursor = 4;
        assert_eq!(
            input.layout(4),
            (vec!["abcd".into(), String::new(), "xy".into()], (0, 1))
        );
    }

    #[test]
    fn emacs_keys_move_and_delete_by_line_and_word() {
        let mut input = Input::new("git commit -m\nfix: the bug".into());
        press(&mut input, KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(input.text(), "git commit -m\nfix: the ");
        press(&mut input, KeyCode::Backspace, KeyModifiers::ALT);
        assert_eq!(input.text(), "git commit -m\nfix: ");
        press(&mut input, KeyCode::Char('b'), KeyModifiers::ALT);
        press(&mut input, KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(input.text(), "git commit -m\n");
        press(&mut input, KeyCode::Left, KeyModifiers::NONE);
        press(&mut input, KeyCode::Left, KeyModifiers::CONTROL);
        assert_eq!(input.cursor, 12);
        press(&mut input, KeyCode::Char('a'), KeyModifiers::CONTROL);
        press(&mut input, KeyCode::Right, KeyModifiers::ALT);
        assert_eq!(input.cursor, 3);
        press(&mut input, KeyCode::Char('f'), KeyModifiers::ALT);
        assert_eq!(input.cursor, 10);
        press(&mut input, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(input.text(), " -m\n");
        press(&mut input, KeyCode::Char('e'), KeyModifiers::CONTROL);
        assert_eq!(input.cursor, 3);
    }

    #[test]
    fn layout_wraps_at_spaces() {
        let input = Input::new("fix the login bug".into());
        assert_eq!(
            input.layout(8),
            (
                vec!["fix the ".into(), "login ".into(), "bug".into()],
                (3, 2)
            )
        );
        // Right after a space that ends a row: the start of the next row.
        let mut input = Input::new("fix the login bug".into());
        input.cursor = 8;
        assert_eq!(input.layout(8).1, (0, 1));
    }
}
