use crate::layout::key::{Key, KeyAction, LayerId};

#[derive(Debug, Clone)]
pub struct KeyboardRow {
    pub keys: Vec<Key>,
}

#[derive(Debug, Clone)]
pub struct KeyboardLayout {
    pub rows: Vec<KeyboardRow>,
    pub id: LayerId,
}

/// Wireframe metric weights (see `keyboard wireframe.md`).
mod flex {
    pub const ESC: f32 = 1.1;
    pub const TAB: f32 = 1.4;
    pub const SHIFT: f32 = 1.8;
    pub const ENTER: f32 = 1.8;
    pub const BACKSPACE: f32 = 1.6;
    pub const TOGGLE: f32 = 1.25;
    pub const MODIFIER: f32 = 1.1; // Ctrl / Alt / Win / Mic / arrows
    pub const SPACE: f32 = 7.2;
}

impl KeyboardLayout {
    pub fn get_layout(id: LayerId, suggestions: &[String]) -> Self {
        match id {
            LayerId::Lower => Self::letters(false, suggestions),
            LayerId::Upper => Self::letters(true, suggestions),
            LayerId::Symbols => Self::symbols(suggestions),
        }
    }

    /// Windows-11 style suggestion / top-bar row.
    ///
    /// When suggestions are present, shows up to 3 pills with the middle one
    /// emphasized. In idle mode the slots are padded with placeholder keys so
    /// the bar keeps its column layout. A dismiss chevron always completes the
    /// bar on the right.
    pub fn make_suggestion_row(suggestions: &[String]) -> KeyboardRow {
        let mut keys = Vec::new();
        if suggestions.is_empty() {
            for idx in 0..3 {
                keys.push(Key::suggestion(idx, "").with_weight(1.0));
            }
        } else {
            for (idx, cand) in suggestions.iter().take(3).enumerate() {
                let weight = if idx == 1 { 1.3 } else { 1.0 };
                let mut k = Key::suggestion(idx, cand).with_weight(weight);
                if idx == 1 {
                    k = k.special();
                }
                keys.push(k);
            }
        }
        keys.push(Key::new("▼", KeyAction::Hide).with_weight(0.5).special());
        KeyboardRow { keys }
    }

    /// Wireframe Layout A: QWERTY letters (`upper` toggles case).
    ///
    /// Row 1: Esc · 1-0 dual digits on qwertyuiop · Backspace
    /// Row 2: Tab · asdfghjkl · '(") · Enter
    /// Row 3: Shift · zxcvbnm · ,(;) .(:) ?(!) · Shift
    /// Row 4: &123 · Ctrl · Win · Alt · Space · Mic · ◀ ▶
    pub fn letters(upper: bool, suggestions: &[String]) -> Self {
        use flex::*;
        let (id, upper) = if upper {
            (LayerId::Upper, true)
        } else {
            (LayerId::Lower, false)
        };
        let text = |c: char| if upper { c.to_ascii_uppercase() } else { c };
        let l = |ch: char| Key::text(text(ch));
        let dual = |ch: char, sec: char| {
            if upper {
                Key::text(text(ch))
            } else {
                Key::text(text(ch)).with_secondary(sec)
            }
        };

        Self {
            id,
            rows: vec![
                Self::make_suggestion_row(suggestions),
                KeyboardRow {
                    keys: vec![
                        Key::new("Esc", KeyAction::Escape).with_weight(ESC).special(),
                        dual('q', '1'),
                        dual('w', '2'),
                        dual('e', '3'),
                        dual('r', '4'),
                        dual('t', '5'),
                        dual('y', '6'),
                        dual('u', '7'),
                        dual('i', '8'),
                        dual('o', '9'),
                        dual('p', '0'),
                        Key::new("⌫", KeyAction::Backspace).with_weight(BACKSPACE).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("Tab", KeyAction::Tab).with_weight(TAB).special(),
                        l('a'),
                        l('s'),
                        l('d'),
                        l('f'),
                        l('g'),
                        l('h'),
                        l('j'),
                        l('k'),
                        l('l'),
                        Key::text(text('\'')).with_secondary("\""),
                        Key::new("⏎", KeyAction::Enter).with_weight(ENTER).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("⇧", KeyAction::Shift).with_weight(SHIFT).special(),
                        l('z'),
                        l('x'),
                        l('c'),
                        l('v'),
                        l('b'),
                        l('n'),
                        l('m'),
                        Key::text(text(',')).with_secondary(";"),
                        Key::text(text('.')).with_secondary(":"),
                        Key::text(text('?')).with_secondary("!"),
                        Key::new("⇧", KeyAction::Shift).with_weight(SHIFT).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("&123", KeyAction::SwitchLayer(LayerId::Symbols)).with_weight(TOGGLE).special(),
                        Key::new("Ctrl", KeyAction::Ctrl).with_weight(MODIFIER).special(),
                        Key::new("⊞", KeyAction::Win).with_weight(MODIFIER).special(),
                        Key::new("Alt", KeyAction::Alt).with_weight(MODIFIER).special(),
                        Key::new("Space", KeyAction::Space).with_weight(SPACE),
                        Key::new("🎤", KeyAction::None).with_weight(MODIFIER).special(),
                        Key::new("◀", KeyAction::ArrowLeft).with_weight(MODIFIER).special(),
                        Key::new("▶", KeyAction::ArrowRight).with_weight(MODIFIER).special(),
                    ],
                },
            ],
        }
    }

    /// Wireframe Layout B: symbols & numbers.
    ///
    /// Row 1: Esc · 1-0 · Backspace
    /// Row 2: Tab · ! @ # $ ^ & _ - = + · Enter
    /// Row 3: ◀ ▶ · ; : ( ) / ' " ? · Home ▲ End
    /// Row 4: abc · Ctrl · Win · Alt · , · Space · . · ◀ ▼ ▶
    pub fn symbols(suggestions: &[String]) -> Self {
        use flex::*;
        Self {
            id: LayerId::Symbols,
            rows: vec![
                Self::make_suggestion_row(suggestions),
                KeyboardRow {
                    keys: vec![
                        Key::new("Esc", KeyAction::Escape).with_weight(ESC).special(),
                        Key::text("1"),
                        Key::text("2"),
                        Key::text("3"),
                        Key::text("4"),
                        Key::text("5"),
                        Key::text("6"),
                        Key::text("7"),
                        Key::text("8"),
                        Key::text("9"),
                        Key::text("0"),
                        Key::new("⌫", KeyAction::Backspace).with_weight(BACKSPACE).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("Tab", KeyAction::Tab).with_weight(TAB).special(),
                        Key::text("!"),
                        Key::text("@"),
                        Key::text("#"),
                        Key::text("$"),
                        Key::text("^"),
                        Key::text("&"),
                        Key::text("_"),
                        Key::text("-"),
                        Key::text("="),
                        Key::text("+"),
                        Key::new("⏎", KeyAction::Enter).with_weight(ENTER).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("◀", KeyAction::ArrowLeft).with_weight(MODIFIER).special(),
                        Key::new("▶", KeyAction::ArrowRight).with_weight(MODIFIER).special(),
                        Key::text(";"),
                        Key::text(":"),
                        Key::text("("),
                        Key::text(")"),
                        Key::text("/"),
                        Key::text("'"),
                        Key::text("\""),
                        Key::text("?"),
                        Key::new("Home", KeyAction::Home).with_weight(TAB).special(),
                        Key::new("▲", KeyAction::ArrowUp).with_weight(MODIFIER).special(),
                        Key::new("End", KeyAction::End).with_weight(TAB).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("abc", KeyAction::SwitchLayer(LayerId::Lower)).with_weight(TOGGLE).special(),
                        Key::new("Ctrl", KeyAction::Ctrl).with_weight(MODIFIER).special(),
                        Key::new("⊞", KeyAction::Win).with_weight(MODIFIER).special(),
                        Key::new("Alt", KeyAction::Alt).with_weight(MODIFIER).special(),
                        Key::text(","),
                        Key::new("Space", KeyAction::Space).with_weight(SPACE),
                        Key::text("."),
                        Key::new("◀", KeyAction::ArrowLeft).with_weight(MODIFIER).special(),
                        Key::new("▼", KeyAction::ArrowDown).with_weight(MODIFIER).special(),
                        Key::new("▶", KeyAction::ArrowRight).with_weight(MODIFIER).special(),
                    ],
                },
            ],
        }
    }
}