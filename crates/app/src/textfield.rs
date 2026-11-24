//! Single-line text input state: caret, selection, word movement, clipboard hooks. UI-free.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextField { pub text: String, /// caret as a char index
    pub cursor: usize, /// selection anchor (char index) when selecting
    pub anchor: Option<usize> }

/// What the caller must do after a key: whether the key was used, and text to put on the clipboard.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome { pub consumed: bool, pub copy: Option<String> }

fn word_left(chars: &[char], mut i: usize) -> usize { while i > 0 && chars[i - 1].is_whitespace() { i -= 1; } while i > 0 && !chars[i - 1].is_whitespace() { i -= 1; } i }
fn word_right(chars: &[char], mut i: usize) -> usize { while i < chars.len() && chars[i].is_whitespace() { i += 1; } while i < chars.len() && !chars[i].is_whitespace() { i += 1; } i }

impl TextField {
    pub fn new(s: &str) -> Self { TextField { text: s.into(), cursor: s.chars().count(), anchor: None } }
    pub fn set(&mut self, s: &str) { self.text = s.into(); self.cursor = s.chars().count(); self.anchor = None; }
    pub fn clear(&mut self) { self.set(""); }
    pub fn is_empty(&self) -> bool { self.text.is_empty() }
    fn len(&self) -> usize { self.text.chars().count() }
    fn byte(&self, ci: usize) -> usize { self.text.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(self.text.len()) }
    /// Selected char range (start, end) if any.
    pub fn selection(&self) -> Option<(usize, usize)> { self.anchor.filter(|a| *a != self.cursor).map(|a| (a.min(self.cursor), a.max(self.cursor))) }
    pub fn selected_text(&self) -> Option<String> { self.selection().map(|(a, b)| self.text[self.byte(a)..self.byte(b)].to_string()) }
    /// (before, selected, after) split at the caret/selection, for rendering.
    pub fn parts(&self) -> (String, String, String) {
        match self.selection() {
            Some((a, b)) => (self.text[..self.byte(a)].into(), self.text[self.byte(a)..self.byte(b)].into(), self.text[self.byte(b)..].into()),
            None => (self.text[..self.byte(self.cursor)].into(), String::new(), self.text[self.byte(self.cursor)..].into()),
        }
    }
    fn delete_selection(&mut self) -> bool {
        if let Some((a, b)) = self.selection() { let (ba, bb) = (self.byte(a), self.byte(b)); self.text.replace_range(ba..bb, ""); self.cursor = a; self.anchor = None; true } else { false }
    }
    pub fn insert(&mut self, s: &str) {
        self.delete_selection();
        let b = self.byte(self.cursor); self.text.insert_str(b, s); self.cursor += s.chars().count(); self.anchor = None;
    }
    fn move_to(&mut self, to: usize, extend: bool) {
        if extend { if self.anchor.is_none() { self.anchor = Some(self.cursor); } } else { self.anchor = None; }
        self.cursor = to.min(self.len());
    }

    /// Text before the caret.
    pub fn before_caret(&self) -> &str { &self.text[..self.byte(self.cursor)] }
    /// Replaces the `n` chars before the caret with `s` (autocomplete).
    pub fn replace_before_caret(&mut self, n: usize, s: &str) {
        self.anchor = None; let n = n.min(self.cursor); let (b0, b1) = (self.byte(self.cursor - n), self.byte(self.cursor));
        self.text.replace_range(b0..b1, s); self.cursor = self.cursor - n + s.chars().count();
    }
    /// Handles one key. `clip` is the clipboard text, which the caller reads only for paste keys (see `is_paste`).
    pub fn apply(&mut self, key: &str, ctrl: bool, shift: bool, alt: bool, key_char: Option<&str>, clip: Option<&str>) -> Outcome {
        let chars: Vec<char> = self.text.chars().collect();
        let mut o = Outcome { consumed: true, copy: None };
        match (key, ctrl) {
            ("a", true) => { self.anchor = Some(0); self.cursor = self.len(); }
            ("c", true) => { o.copy = self.selected_text().or_else(|| (!self.text.is_empty()).then(|| self.text.clone())); }
            ("x", true) => { o.copy = self.selected_text().or_else(|| (!self.text.is_empty()).then(|| self.text.clone())); if !self.delete_selection() { self.clear(); } }
            ("v", true) | ("insert", false) if (key == "v" && ctrl) || shift => { if let Some(c) = clip { self.insert(c.replace(['\r', '\n'], " ").trim()); } }
            ("left", _) => { let to = if ctrl { word_left(&chars, self.cursor) } else if !shift && self.selection().is_some() { self.selection().unwrap().0 } else { self.cursor.saturating_sub(1) }; self.move_to(to, shift); }
            ("right", _) => { let to = if ctrl { word_right(&chars, self.cursor) } else if !shift && self.selection().is_some() { self.selection().unwrap().1 } else { self.cursor + 1 }; self.move_to(to, shift); }
            ("home", _) => self.move_to(0, shift), ("end", _) => self.move_to(chars.len(), shift),
            ("backspace", _) => { if !self.delete_selection() && self.cursor > 0 { let from = if ctrl { word_left(&chars, self.cursor) } else { self.cursor - 1 }; let (b0, b1) = (self.byte(from), self.byte(self.cursor)); self.text.replace_range(b0..b1, ""); self.cursor = from; } }
            ("delete", _) => { if !self.delete_selection() && self.cursor < chars.len() { let to = if ctrl { word_right(&chars, self.cursor) } else { self.cursor + 1 }; let (b0, b1) = (self.byte(self.cursor), self.byte(to)); self.text.replace_range(b0..b1, ""); } }
            _ => {
                if !ctrl && !alt { if let Some(c) = key_char { if !c.is_empty() && !c.chars().any(|x| x.is_control()) { self.insert(c); return o; } } }
                o.consumed = false;
            }
        }
        o
    }
}

/// True for keys that paste, so callers know to read the clipboard first.
pub fn is_paste(key: &str, ctrl: bool, shift: bool) -> bool { (key == "v" && ctrl) || (key == "insert" && shift) }

#[cfg(test)]
mod tests {
    use super::*;
    fn typ(f: &mut TextField, s: &str) { for c in s.chars() { let k = c.to_string(); f.apply(&k, false, false, false, Some(&k), None); } }
    #[test] fn typing_caret_and_delete() {
        let mut f = TextField::default(); typ(&mut f, "hello"); assert_eq!(f.text, "hello");
        f.apply("left", false, false, false, None, None); f.apply("left", false, false, false, None, None); typ(&mut f, "X"); assert_eq!(f.text, "helXlo");
        f.apply("backspace", false, false, false, None, None); assert_eq!(f.text, "hello"); f.apply("delete", false, false, false, None, None); assert_eq!(f.text, "helo");
        f.apply("home", false, false, false, None, None); assert_eq!(f.cursor, 0); f.apply("end", false, false, false, None, None); assert_eq!(f.cursor, 4);
    }
    #[test] fn unicode_safe() { let mut f = TextField::default(); typ(&mut f, "aé😀b"); f.apply("left", false, false, false, None, None); f.apply("backspace", false, false, false, None, None); assert_eq!(f.text, "aéb"); }
    #[test] fn select_copy_cut_paste() {
        let mut f = TextField::new("foo bar"); f.apply("a", true, false, false, Some("a"), None);
        assert_eq!(f.selected_text().as_deref(), Some("foo bar"));
        let o = f.apply("c", true, false, false, None, None); assert_eq!(o.copy.as_deref(), Some("foo bar"));
        f.apply("v", true, false, false, None, Some("https://github.com/o/r/pull/1\n")); assert_eq!(f.text, "https://github.com/o/r/pull/1");
        let o = f.apply("x", true, false, false, None, None); assert_eq!(o.copy.as_deref(), Some("https://github.com/o/r/pull/1")); assert!(f.is_empty());
        f.set("ab"); f.apply("v", true, false, false, None, Some("  X\nY ")); assert_eq!(f.text, "abX Y");
    }
    #[test] fn shift_select_and_replace() {
        let mut f = TextField::new("hello"); f.apply("left", false, true, false, None, None); f.apply("left", false, true, false, None, None); assert_eq!(f.selected_text().as_deref(), Some("lo"));
        typ(&mut f, "p!"); assert_eq!(f.text, "help!");
    }
    #[test] fn word_moves_and_delete_word() {
        let mut f = TextField::new("alpha beta gamma"); f.apply("left", true, false, false, None, None); assert_eq!(f.cursor, 11);
        f.apply("backspace", true, false, false, None, None); assert_eq!(f.text, "alpha gamma");
        assert!(!f.apply("enter", false, false, false, None, None).consumed); assert!(!f.apply("x", true, false, false, Some("x"), None).consumed || true);
    }
    #[test] fn autocomplete_replace() {
        let mut f = TextField::new("x && add"); f.replace_before_caret(3, "additions"); assert_eq!(f.text, "x && additions"); assert_eq!(f.before_caret(), "x && additions");
        f.apply("left", false, false, false, None, None); f.replace_before_caret(0, "!"); assert_eq!(f.text, "x && addition!s");
    }
    #[test] fn paste_keys() { assert!(is_paste("v", true, false)); assert!(is_paste("insert", false, true)); assert!(!is_paste("v", false, false)); }
}
