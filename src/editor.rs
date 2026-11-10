//! The reply box's text, edited like a text field: a cursor, word jumps, a selection to replace,
//! copy or cut. Positions count characters, not bytes.

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Editor {
    pub text: String,
    /// where typing goes, in characters
    pub cursor: usize,
    /// the other end of a selection, while there is one
    pub anchor: Option<usize>,
}

impl Editor {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, i: usize) -> usize {
        self.text
            .char_indices()
            .nth(i)
            .map(|(b, _)| b)
            .unwrap_or(self.text.len())
    }

    /// The selection, start to end, when there is one.
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.anchor
            .filter(|&a| a != self.cursor)
            .map(|a| (a.min(self.cursor), a.max(self.cursor)))
    }

    fn delete_selection(&mut self) -> bool {
        let sel = self.selection();
        self.anchor = None;
        match sel {
            Some((a, b)) => {
                let (ba, bb) = (self.byte(a), self.byte(b));
                self.text.replace_range(ba..bb, "");
                self.cursor = a;
                true
            }
            None => false,
        }
    }

    /// Typing (or a paste): it replaces the selection, at the cursor.
    pub fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        let b = self.byte(self.cursor);
        self.text.insert_str(b, s);
        self.cursor += s.chars().count();
    }

    pub fn insert(&mut self, c: char) {
        self.insert_str(c.encode_utf8(&mut [0; 4]));
    }

    pub fn backspace(&mut self) {
        if !self.delete_selection() && self.cursor > 0 {
            let (a, b) = (self.byte(self.cursor - 1), self.byte(self.cursor));
            self.text.replace_range(a..b, "");
            self.cursor -= 1;
        }
    }

    pub fn delete(&mut self) {
        if !self.delete_selection() && self.cursor < self.len() {
            let (a, b) = (self.byte(self.cursor), self.byte(self.cursor + 1));
            self.text.replace_range(a..b, "");
        }
    }

    fn move_to(&mut self, to: usize, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = to.min(self.len());
    }

    pub fn left(&mut self, word: bool, extend: bool) {
        let to = if word {
            self.word_start()
        } else {
            self.cursor.saturating_sub(1)
        };
        self.move_to(to, extend);
    }

    pub fn right(&mut self, word: bool, extend: bool) {
        let to = if word {
            self.word_end()
        } else {
            self.cursor + 1
        };
        self.move_to(to, extend);
    }

    pub fn home(&mut self, extend: bool) {
        self.move_to(0, extend);
    }

    pub fn end(&mut self, extend: bool) {
        let n = self.len();
        self.move_to(n, extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.len();
    }

    pub fn selected(&self) -> Option<String> {
        self.selection()
            .map(|(a, b)| self.text.chars().skip(a).take(b - a).collect())
    }

    pub fn cut(&mut self) -> Option<String> {
        let s = self.selected();
        self.delete_selection();
        s
    }

    pub fn clear(&mut self) {
        *self = Editor::default();
    }

    /// The text, leaving the box empty.
    pub fn take(&mut self) -> String {
        let t = std::mem::take(&mut self.text);
        self.clear();
        t
    }

    fn word_start(&self) -> usize {
        let c: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor.min(c.len());
        while i > 0 && c[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !c[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    fn word_end(&self) -> usize {
        let c: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor.min(c.len());
        while i < c.len() && c[i].is_whitespace() {
            i += 1;
        }
        while i < c.len() && !c[i].is_whitespace() {
            i += 1;
        }
        i
    }

    /// The text as shown: the cursor mark at the cursor.
    pub fn display(&self) -> String {
        let b = self.byte(self.cursor);
        format!("{}\u{258f}{}", &self.text[..b], &self.text[b..])
    }

    /// The selection's place in `display()` (the cursor mark shifts what follows it).
    pub fn selection_in_display(&self) -> Option<(usize, usize)> {
        self.selection().map(|(a, b)| {
            if self.cursor == a {
                (a + 1, b + 1)
            } else {
                (a, b)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> Editor {
        let mut e = Editor::default();
        e.insert_str(s);
        e
    }

    #[test]
    fn typing_and_deleting_happen_at_the_cursor() {
        let mut e = typed("helo");
        e.left(false, false);
        e.insert('l');
        assert_eq!((e.text.as_str(), e.cursor), ("hello", 4));
        e.home(false);
        e.delete();
        assert_eq!(e.text, "ello");
        e.end(false);
        e.backspace();
        assert_eq!(e.text, "ell");
    }

    #[test]
    fn words_are_jumped_over() {
        let mut e = typed("one two  three");
        e.left(true, false);
        assert_eq!(e.cursor, 9, "to the start of three");
        e.left(true, false);
        assert_eq!(e.cursor, 4, "to the start of two");
        e.right(true, false);
        assert_eq!(e.cursor, 7, "to the end of two");
    }

    #[test]
    fn a_selection_is_replaced_copied_or_cut() {
        let mut e = typed("hello world");
        e.left(true, true);
        assert_eq!(e.selected().as_deref(), Some("world"));
        e.insert_str("there");
        assert_eq!(e.text, "hello there");
        e.select_all();
        assert_eq!(e.cut().as_deref(), Some("hello there"));
        assert!(e.is_empty());
    }

    #[test]
    fn the_cursor_mark_and_the_selection_show_where_they_are() {
        let mut e = typed("abcd");
        e.left(false, false);
        e.left(false, false);
        assert_eq!(e.display(), "ab\u{258f}cd");
        e.right(false, true);
        assert_eq!(
            e.selection_in_display(),
            Some((2, 3)),
            "the c, before the mark that follows it"
        );
        e.left(false, false);
        e.left(false, true);
        assert_eq!(
            e.selection_in_display(),
            Some((2, 3)),
            "the b, after the mark before it"
        );
    }

    #[test]
    fn non_ascii_text_is_edited_by_characters() {
        let mut e = typed("привіт");
        e.backspace();
        e.left(false, false);
        e.insert('ї');
        assert_eq!(e.text, "привїі");
    }
}
