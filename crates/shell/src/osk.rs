//! On-screen keyboard model, laid out like the Xbox Series X|S keyboard:
//!
//! ```text
//! [#+=]  1 2 3 4 5 6 7 8 9 0  [ ⌫ ]
//! [ ← ]  q w e r t y u i o p  [ → ]
//! [   ]  a s d f g h j k l "  [   ]
//! [ ⇧ ]  z x c v b n m ; : !  [Done]
//! [   ]  [       space       ]  [    ]
//! ```
//!
//! Keys have geometry (in key units) so focus moves spatially across the tall side keys.
//! Controller: A = press key, X = backspace, Y = space, LB/RB = move the cursor, Menu = done, B = cancel.
//! A physical keyboard can type directly while the keyboard is open.

use cs_input::NavAction;

use crate::nav::{Rect, spatial_move};

const LOWER: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl\"", "zxcvbnm;:!"];
const SYMBOLS: [&str; 4] = ["1234567890", "@#$%&*()-_", "+=[]{}<>/\\", "',.?~`^|!\""];

/// Width of the side columns, in key units.
const SIDE: f32 = 1.7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Char(char),
    Symbols,
    Backspace,
    CursorLeft,
    CursorRight,
    Shift,
    Space,
    Done,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    /// Position and size in key units (a letter key is 1×1).
    pub rect: Rect,
    pub label: String,
    /// Controller shortcut shown in the key's corner ("X", "Y", "LB", "RB", "≡"), or "".
    pub hint: &'static str,
    pub action: KeyAction,
    /// Toggle keys (Shift, symbols) that are currently on.
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OskOutcome {
    Continue,
    Done(String),
    Cancel,
}

#[derive(Debug, Clone)]
pub struct Osk {
    pub title: String,
    pub placeholder: String,
    text: Vec<char>,
    /// Insertion point, in chars.
    cursor: usize,
    pub max_len: usize,
    shift: bool,
    symbols: bool,
    /// Index into `keys()`.
    pub focus: usize,
}

impl Osk {
    pub fn new(title: impl Into<String>, initial: &str, max_len: usize) -> Self {
        let text: Vec<char> = initial.chars().take(max_len).collect();
        let mut osk = Self {
            title: title.into(),
            placeholder: String::new(),
            cursor: text.len(),
            shift: text.is_empty(), // capitalise the first letter, like a console keyboard
            text,
            max_len,
            symbols: false,
            focus: 0,
        };
        osk.focus = osk.keys().iter().position(|k| matches!(k.action, KeyAction::Char('q' | 'Q'))).unwrap_or(0);
        osk
    }

    pub fn text(&self) -> String {
        self.text.iter().collect()
    }

    /// Text split at the cursor, for rendering the caret.
    pub fn text_around_cursor(&self) -> (String, String) {
        (self.text[..self.cursor].iter().collect(), self.text[self.cursor..].iter().collect())
    }

    /// All keys with their geometry, for the current shift/symbol state.
    pub fn keys(&self) -> Vec<Key> {
        let mut keys = Vec::with_capacity(48);
        let mut side = |x: f32, y: f32, h: f32, label: &str, hint: &'static str, action: KeyAction, active: bool| {
            keys.push(Key { rect: Rect::new(x, y, SIDE, h), label: label.into(), hint, action, active });
        };
        let right = SIDE + 10.0;
        side(0.0, 0.0, 1.0, if self.symbols { "abc" } else { "#+=" }, "", KeyAction::Symbols, self.symbols);
        side(0.0, 1.0, 2.0, "←", "LB", KeyAction::CursorLeft, false);
        side(0.0, 3.0, 2.0, "⇧", "", KeyAction::Shift, self.shift);
        side(right, 0.0, 1.0, "⌫", "X", KeyAction::Backspace, false);
        side(right, 1.0, 2.0, "→", "RB", KeyAction::CursorRight, false);
        side(right, 3.0, 2.0, "Done", "≡", KeyAction::Done, false);

        let layout = if self.symbols { &SYMBOLS } else { &LOWER };
        for (row, chars) in layout.iter().enumerate() {
            for (col, c) in chars.chars().enumerate() {
                let shown = self.display_char(c);
                keys.push(Key {
                    rect: Rect::new(SIDE + col as f32, row as f32, 1.0, 1.0),
                    label: shown.to_string(),
                    hint: "",
                    action: KeyAction::Char(shown),
                    active: false,
                });
            }
        }
        keys.push(Key {
            rect: Rect::new(SIDE, 4.0, 10.0, 1.0),
            label: "Space".into(),
            hint: "Y",
            action: KeyAction::Space,
            active: false,
        });
        keys
    }

    /// Whether focus is on the top row (Search moves up to the results from here).
    pub fn focus_on_top_row(&self) -> bool {
        self.keys().get(self.focus).is_some_and(|k| k.rect.y == 0.0)
    }

    fn display_char(&self, c: char) -> char {
        if self.shift && !self.symbols { c.to_ascii_uppercase() } else { c }
    }

    pub fn handle(&mut self, action: NavAction) -> OskOutcome {
        match action {
            NavAction::Up | NavAction::Down | NavAction::Left | NavAction::Right => {
                let rects: Vec<Rect> = self.keys().iter().map(|k| k.rect).collect();
                if let Some(f) = spatial_move(&rects, self.focus, action) {
                    self.focus = f;
                }
            }
            NavAction::Accept => return self.press(self.focus),
            NavAction::Back => return OskOutcome::Cancel,
            NavAction::X => self.backspace(),
            NavAction::Y => self.type_char(' '),
            NavAction::PrevTab => self.cursor = self.cursor.saturating_sub(1),
            NavAction::NextTab => self.cursor = (self.cursor + 1).min(self.text.len()),
            NavAction::Menu => return OskOutcome::Done(self.text().trim().to_string()),
            _ => {}
        }
        OskOutcome::Continue
    }

    /// Presses key `index` (from the controller or a mouse click).
    pub fn press(&mut self, index: usize) -> OskOutcome {
        let Some(key) = self.keys().into_iter().nth(index) else { return OskOutcome::Continue };
        self.focus = index;
        match key.action {
            KeyAction::Char(c) => self.type_char(c),
            KeyAction::Symbols => self.symbols = !self.symbols,
            KeyAction::Backspace => self.backspace(),
            KeyAction::CursorLeft => self.cursor = self.cursor.saturating_sub(1),
            KeyAction::CursorRight => self.cursor = (self.cursor + 1).min(self.text.len()),
            KeyAction::Shift => self.shift = !self.shift,
            KeyAction::Space => self.type_char(' '),
            KeyAction::Done => return OskOutcome::Done(self.text().trim().to_string()),
        }
        OskOutcome::Continue
    }

    /// Inserts a character at the cursor (from a key or the physical keyboard). Shift is one-shot.
    pub fn type_char(&mut self, c: char) {
        if c.is_control() || self.text.len() >= self.max_len {
            return;
        }
        self.text.insert(self.cursor, c);
        self.cursor += 1;
        if c != ' ' {
            self.shift = false;
        }
    }

    /// Empties the text (the virtual keyboard sends each key on and keeps nothing).
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.text.remove(self.cursor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_of(k: &Osk, action: KeyAction) -> usize {
        k.keys().iter().position(|key| key.action == action).unwrap()
    }

    #[test]
    fn first_letter_is_capitalised_and_shift_is_one_shot() {
        let mut k = Osk::new("Name", "", 10);
        k.press(index_of(&k, KeyAction::Char('Q')));
        k.press(index_of(&k, KeyAction::Char('w')));
        assert_eq!(k.text(), "Qw");
        k.handle(NavAction::Y);
        k.handle(NavAction::X);
        k.handle(NavAction::X);
        assert_eq!(k.text(), "Q");
        assert_eq!(k.handle(NavAction::Menu), OskOutcome::Done("Q".into()));
        assert_eq!(k.handle(NavAction::Back), OskOutcome::Cancel);
    }

    #[test]
    fn cursor_editing() {
        let mut k = Osk::new("x", "acd", 10);
        k.handle(NavAction::PrevTab);
        k.handle(NavAction::PrevTab);
        k.type_char('b');
        assert_eq!(k.text_around_cursor(), ("ab".into(), "cd".into()));
        k.handle(NavAction::X);
        assert_eq!(k.text(), "acd");
        for _ in 0..5 {
            k.handle(NavAction::NextTab);
        }
        k.type_char('e');
        assert_eq!(k.text(), "acde");
    }

    #[test]
    fn max_len_and_symbols() {
        let mut k = Osk::new("Hex", "#12345", 7);
        k.press(index_of(&k, KeyAction::Symbols));
        let hash = index_of(&k, KeyAction::Char('#'));
        k.press(hash);
        k.press(hash);
        assert_eq!(k.text(), "#12345#");
    }

    #[test]
    fn spatial_movement_over_tall_keys() {
        let mut k = Osk::new("x", "x", 10);
        assert_eq!(k.keys()[k.focus].action, KeyAction::Char('q'));
        k.handle(NavAction::Left);
        assert_eq!(k.keys()[k.focus].action, KeyAction::CursorLeft); // tall key spanning q and a rows
        k.handle(NavAction::Down);
        assert_eq!(k.keys()[k.focus].action, KeyAction::Shift);
        k.handle(NavAction::Right);
        assert_eq!(k.keys()[k.focus].action, KeyAction::Char('z'));
        k.handle(NavAction::Down);
        assert_eq!(k.keys()[k.focus].action, KeyAction::Space);
        k.handle(NavAction::Right);
        assert_eq!(k.keys()[k.focus].action, KeyAction::Done);
        k.handle(NavAction::Up);
        assert_eq!(k.keys()[k.focus].action, KeyAction::CursorRight);
        k.handle(NavAction::Up);
        assert_eq!(k.keys()[k.focus].action, KeyAction::Backspace);
    }
}
