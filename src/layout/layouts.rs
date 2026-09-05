use crate::layout::key::{Key, KeyAction, LayerId, LayoutMode};

#[derive(Debug, Clone, PartialEq)]
pub struct KeyboardRow {
    pub keys: Vec<Key>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyboardLayout {
    pub rows: Vec<KeyboardRow>,
    pub id: LayerId,
}

/// Windows 11 OSK metric weights (see `keyboard wireframe-2.md`).
mod flex {
    pub const ESC: f32 = 1.1;
    pub const TAB: f32 = 1.5;
    pub const SHIFT: f32 = 2.3;
    pub const ENTER: f32 = 2.1;
    pub const BACKSPACE: f32 = 1.5;
    pub const TOGGLE: f32 = 1.25;
    pub const SYMPAGE: f32 = 2.0;
    pub const MODIFIER: f32 = 1.1; // Ctrl / Alt / Win / Mic / arrows
    pub const SPACE: f32 = 7.2;
}

impl KeyboardLayout {
    /// Returns true if this layout is the mobile (HeliBoard / Gboard) layout.
    /// In mobile layout, row 1 contains 10 letter keys without Esc.
    pub fn is_mobile(&self) -> bool {
        self.rows
            .get(1)
            .map(|r| r.keys.len() == 10 && !r.keys[0].is_special)
            .unwrap_or(false)
    }

    pub fn get_layout_with_caps(id: LayerId, suggestions: &[String], caps_lock: bool) -> Self {
        match id {
            LayerId::Lower => Self::letters(false, suggestions, false),
            LayerId::Upper => Self::letters(true, suggestions, caps_lock),
            LayerId::Symbols => Self::symbols_page1(suggestions),
            LayerId::Symbols2 => Self::symbols_page2(suggestions),
        }
    }

    pub fn get_portrait_layout_with_caps(id: LayerId, suggestions: &[String], caps_lock: bool) -> Self {
        match id {
            LayerId::Lower => Self::letters_portrait(false, suggestions, false),
            LayerId::Upper => Self::letters_portrait(true, suggestions, caps_lock),
            LayerId::Symbols => Self::symbols_page1_portrait(suggestions),
            LayerId::Symbols2 => Self::symbols_page2_portrait(suggestions),
        }
    }

    pub fn get_layout_for_mode(id: LayerId, suggestions: &[String], caps_lock: bool, mode: LayoutMode) -> Self {
        match mode {
            LayoutMode::Mobile => Self::get_portrait_layout_with_caps(id, suggestions, caps_lock),
            LayoutMode::Desktop => Self::get_layout_with_caps(id, suggestions, caps_lock),
        }
    }

    pub fn get_layout_for_size(id: LayerId, suggestions: &[String], caps_lock: bool, width: u32, height: u32) -> Self {
        let is_portrait = width < 1000 || (width as f32 / height.max(1) as f32) < 1.25;
        let mode = if is_portrait { LayoutMode::Mobile } else { LayoutMode::Desktop };
        Self::get_layout_for_mode(id, suggestions, caps_lock, mode)
    }

    pub fn get_layout(id: LayerId, suggestions: &[String]) -> Self {
        Self::get_layout_with_caps(id, suggestions, false)
    }

    /// A 46px action bar replacing the old suggestion row.
    ///
    /// Left: Settings (gear) and Theme (palette) icons. Center: up to 3
    /// auto-suggestion pills shown only while the user is typing a word --
    /// when idle the slots stay in the model (same icon geometry) but render
    /// invisible and swallow no input. Right: clipboard history toggle and the
    /// dismiss chevron (minimizes the keyboard).
    pub fn make_top_bar(suggestions: &[String]) -> KeyboardRow {
        let mut keys = Vec::new();
        let action_weight = 0.35;
        keys.push(Key::new("gear", KeyAction::None).with_weight(action_weight).special());
        keys.push(Key::new("palette", KeyAction::None).with_weight(action_weight).special());

        let s0 = suggestions.first().map(|s| s.as_str()).unwrap_or("");
        let s1 = suggestions.get(1).map(|s| s.as_str()).unwrap_or("");
        let s2 = suggestions.get(2).map(|s| s.as_str()).unwrap_or("");

        keys.push(Key::suggestion(0, s0).with_weight(1.0));
        keys.push(Key::suggestion(1, s1).with_weight(1.3));
        keys.push(Key::suggestion(2, s2).with_weight(1.0));

        keys.push(Key::new("", KeyAction::Clipboard).with_weight(action_weight).special());
        keys.push(Key::new("", KeyAction::Hide).with_weight(action_weight).special());
        KeyboardRow { keys }
    }

    /// The top bar shared by every layer.
    fn with_top_bar(mut self, suggestions: &[String]) -> Self {
        self.rows.insert(0, Self::make_top_bar(suggestions));
        self
    }

    /// Windows-11 style letter layout (`wireframe-2.md` View 2).
    ///
    /// Row 1: Esc · q-p + digit sub-chars · Backspace
    /// Row 2: Tab · a-l + symbol sub-chars · Enter
    /// Row 3: Shift · z-m + symbol sub-chars · Shift
    /// Row 4: &123 · Ctrl · Win · Alt · Space · Mic · ◀ ▶
    pub fn letters(upper: bool, suggestions: &[String], caps_lock: bool) -> Self {
        use flex::*;
        let (id, upper) = if upper {
            (LayerId::Upper, true)
        } else {
            (LayerId::Lower, false)
        };
        let text = |c: char| if upper { c.to_ascii_uppercase() } else { c };
        let dual = |ch: char, sec: char| {
            if upper {
                Key::text(text(ch))
            } else {
                Key::text(text(ch)).with_secondary(sec)
            }
        };
        let shift_label = if caps_lock { "⇪" } else { "⇧" };

        Self {
            id,
            rows: vec![
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
                        dual('a', '@'),
                        dual('s', '#'),
                        dual('d', '$'),
                        dual('f', '%'),
                        dual('g', '&'),
                        dual('h', '-'),
                        dual('j', '+'),
                        dual('k', '('),
                        dual('l', ')'),
                        Key::new("⏎", KeyAction::Enter).with_weight(ENTER).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new(shift_label, KeyAction::Shift).with_weight(SHIFT).special(),
                        dual('z', '*'),
                        dual('x', '"'),
                        dual('c', '\''),
                        dual('v', ':'),
                        dual('b', ';'),
                        dual('n', '!'),
                        dual('m', '?'),
                        Key::new(shift_label, KeyAction::Shift).with_weight(SHIFT).special(),
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
        .with_top_bar(suggestions)
    }

    /// Symbols page 1: numbers & primary symbols (`wireframe-2.md` View 3).
    ///
    /// Row 1: Esc · 1-0 · Backspace
    /// Row 2: Tab · ! @ # $ ^ & _ - = + · Enter
    /// Row 3: =\< (page switcher) · ; : ( ) / ' " ? · Home ↑ End
    /// Row 4: abc · Ctrl · Win · Alt · , · Space · . · ◀ ▼ ▶
    pub fn symbols_page1(suggestions: &[String]) -> Self {
        use flex::*;
        Self {
            id: LayerId::Symbols,
            rows: vec![
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
                        Key::new("=\\<", KeyAction::SwitchLayer(LayerId::Symbols2)).with_weight(SYMPAGE).special(),
                        Key::text(";"),
                        Key::text(":"),
                        Key::text("("),
                        Key::text(")"),
                        Key::text("/"),
                        Key::text("'"),
                        Key::text("\""),
                        Key::text("?"),
                        Key::new("Home", KeyAction::Home).special(),
                        Key::new("▲", KeyAction::ArrowUp).with_weight(MODIFIER).special(),
                        Key::new("End", KeyAction::End).special(),
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
        .with_top_bar(suggestions)
    }

    /// Symbols page 2: brackets, currenices & extended symbols
    /// (`wireframe-2.md` View 4).
    ///
    /// Row 1: Esc · ~ ` | [ ] { } < > \ · Backspace
    /// Row 2: Tab · € £ ¥ ¢ ₹ § ± × ÷ ≠ · Enter
    /// Row 3: 123 (page switcher) · ° • © ® ™ « » ¿ · Home ↑ End
    /// Row 4: abc · Ctrl · Win · Alt · , · Space · . · ◀ ▼ ▶
    pub fn symbols_page2(suggestions: &[String]) -> Self {
        use flex::*;
        Self {
            id: LayerId::Symbols2,
            rows: vec![
                KeyboardRow {
                    keys: vec![
                        Key::new("Esc", KeyAction::Escape).with_weight(ESC).special(),
                        Key::text("~"),
                        Key::text("`"),
                        Key::text("|"),
                        Key::text("["),
                        Key::text("]"),
                        Key::text("{"),
                        Key::text("}"),
                        Key::text("<"),
                        Key::text(">"),
                        Key::text("\\"),
                        Key::new("⌫", KeyAction::Backspace).with_weight(BACKSPACE).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("Tab", KeyAction::Tab).with_weight(TAB).special(),
                        Key::text("€"),
                        Key::text("£"),
                        Key::text("¥"),
                        Key::text("¢"),
                        Key::text("₹"),
                        Key::text("§"),
                        Key::text("±"),
                        Key::text("×"),
                        Key::text("÷"),
                        Key::text("≠"),
                        Key::new("⏎", KeyAction::Enter).with_weight(ENTER).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("123", KeyAction::SwitchLayer(LayerId::Symbols)).with_weight(SYMPAGE).special(),
                        Key::text("°"),
                        Key::text("•"),
                        Key::text("©"),
                        Key::text("®"),
                        Key::text("™"),
                        Key::text("«"),
                        Key::text("»"),
                        Key::text("¿"),
                        Key::new("Home", KeyAction::Home).special(),
                        Key::new("▲", KeyAction::ArrowUp).with_weight(MODIFIER).special(),
                        Key::new("End", KeyAction::End).special(),
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
        .with_top_bar(suggestions)
    }

    /// Clipboard history view replacing the key rows (`wireframe-2.md` View 1).
    ///
    /// Row 1: ◀ Back · header
    /// Rows 2-3: up to 8 history entries presented as suggestion pills;
    /// tapping an entry pastes it.
    #[allow(clippy::needless_lifetimes)]
    pub fn clipboard(history: &[String], suggestions: &[String]) -> Self {
        let top = vec![
            Key::new("◀ Back", KeyAction::Clipboard).with_weight(0.8).special().clipboard(),
            Key::new("Clipboard History", KeyAction::None).with_weight(3.0).special().clipboard(),
        ];

        let mut rows = vec![KeyboardRow { keys: top }];
        let mut idx = 0usize;
        for chunk in history.chunks(4) {
            let row_keys: Vec<Key> = chunk
                .iter()
                .map(|text| {
                    let label = clipboard_label(text);
                    let mut key = Key::new(label, KeyAction::ClipboardItem(idx)).special().clipboard();
                    if idx == 0 {
                        key = key.pinned();
                    }
                    idx += 1;
                    key
                })
                .collect();
            rows.push(KeyboardRow { keys: row_keys });
        }

        Self { id: LayerId::Lower, rows }.with_top_bar(suggestions)
    }

    /// HeliBoard / Gboard portrait layout: Clean mobile 4-row layout without desktop keys.
    ///
    /// Row 1: q w e r t y u i o p (dual secondary: 1..0)
    /// Row 2: a s d f g h j k l (dual secondary: @ # $ % & - + ( ))
    /// Row 3: Shift (1.35) · z x c v b n m · ⌫ (1.35)
    /// Row 4: ?123 (1.35) · , (1.0) · Space (4.9) · . (1.0) · ⏎ (1.35)
    pub fn letters_portrait(upper: bool, suggestions: &[String], caps_lock: bool) -> Self {
        let (id, upper) = if upper {
            (LayerId::Upper, true)
        } else {
            (LayerId::Lower, false)
        };
        let text = |c: char| if upper { c.to_ascii_uppercase() } else { c };
        let dual = |ch: char, sec: char| {
            if upper {
                Key::text(text(ch))
            } else {
                Key::text(text(ch)).with_secondary(sec)
            }
        };
        let shift_label = if caps_lock { "⇪" } else { "⇧" };

        let side_key_w = 1.35;
        let p_space_w = 4.9;

        Self {
            id,
            rows: vec![
                KeyboardRow {
                    keys: vec![
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
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        dual('a', '@'),
                        dual('s', '#'),
                        dual('d', '$'),
                        dual('f', '%'),
                        dual('g', '&'),
                        dual('h', '-'),
                        dual('j', '+'),
                        dual('k', '('),
                        dual('l', ')'),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new(shift_label, KeyAction::Shift).with_weight(side_key_w).special(),
                        dual('z', '*'),
                        dual('x', '"'),
                        dual('c', '\''),
                        dual('v', ':'),
                        dual('b', ';'),
                        dual('n', '!'),
                        dual('m', '?'),
                        Key::new("⌫", KeyAction::Backspace).with_weight(side_key_w).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("?123", KeyAction::SwitchLayer(LayerId::Symbols)).with_weight(side_key_w).special(),
                        Key::text(",").with_weight(1.0),
                        Key::new("Space", KeyAction::Space).with_weight(p_space_w),
                        Key::text(".").with_weight(1.0),
                        Key::new("⏎", KeyAction::Enter).with_weight(side_key_w).special(),
                    ],
                },
            ],
        }
        .with_top_bar(suggestions)
    }

    /// HeliBoard / Gboard portrait symbols page 1 (numbers & primary symbols).
    ///
    /// Row 1: 1 2 3 4 5 6 7 8 9 0
    /// Row 2: @ # $ % & - + ( ) /
    /// Row 3: =\< (1.35) · * " ' : ; ! ? · ⌫ (1.35)
    /// Row 4: abc (1.35) · , (1.0) · Space (4.9) · . (1.0) · ⏎ (1.35)
    pub fn symbols_page1_portrait(suggestions: &[String]) -> Self {
        let side_key_w = 1.35;
        let p_space_w = 4.9;

        Self {
            id: LayerId::Symbols,
            rows: vec![
                KeyboardRow {
                    keys: vec![
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
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::text("@"),
                        Key::text("#"),
                        Key::text("$"),
                        Key::text("%"),
                        Key::text("&"),
                        Key::text("-"),
                        Key::text("+"),
                        Key::text("("),
                        Key::text(")"),
                        Key::text("/"),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("=\\<", KeyAction::SwitchLayer(LayerId::Symbols2)).with_weight(side_key_w).special(),
                        Key::text("*"),
                        Key::text("\""),
                        Key::text("'"),
                        Key::text(":"),
                        Key::text(";"),
                        Key::text("!"),
                        Key::text("?"),
                        Key::new("⌫", KeyAction::Backspace).with_weight(side_key_w).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("abc", KeyAction::SwitchLayer(LayerId::Lower)).with_weight(side_key_w).special(),
                        Key::text(",").with_weight(1.0),
                        Key::new("Space", KeyAction::Space).with_weight(p_space_w),
                        Key::text(".").with_weight(1.0),
                        Key::new("⏎", KeyAction::Enter).with_weight(side_key_w).special(),
                    ],
                },
            ],
        }
        .with_top_bar(suggestions)
    }

    /// HeliBoard / Gboard portrait symbols page 2 (brackets & extended symbols).
    ///
    /// Row 1: ~ ` | ^ _ = < > [ ]
    /// Row 2: { } € £ ¥ ¢ ₹ ° • \
    /// Row 3: 123 (1.35) · © ® ™ « » ± × ÷ · ⌫ (1.35)
    /// Row 4: abc (1.35) · , (1.0) · Space (4.9) · . (1.0) · ⏎ (1.35)
    pub fn symbols_page2_portrait(suggestions: &[String]) -> Self {
        let side_key_w = 1.35;
        let p_space_w = 4.9;

        Self {
            id: LayerId::Symbols2,
            rows: vec![
                KeyboardRow {
                    keys: vec![
                        Key::text("~"),
                        Key::text("`"),
                        Key::text("|"),
                        Key::text("^"),
                        Key::text("_"),
                        Key::text("="),
                        Key::text("<"),
                        Key::text(">"),
                        Key::text("["),
                        Key::text("]"),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::text("{"),
                        Key::text("}"),
                        Key::text("€"),
                        Key::text("£"),
                        Key::text("¥"),
                        Key::text("¢"),
                        Key::text("₹"),
                        Key::text("°"),
                        Key::text("•"),
                        Key::text("\\"),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("123", KeyAction::SwitchLayer(LayerId::Symbols)).with_weight(side_key_w).special(),
                        Key::text("©"),
                        Key::text("®"),
                        Key::text("™"),
                        Key::text("«"),
                        Key::text("»"),
                        Key::text("±"),
                        Key::text("×"),
                        Key::text("÷"),
                        Key::new("⌫", KeyAction::Backspace).with_weight(side_key_w).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("abc", KeyAction::SwitchLayer(LayerId::Lower)).with_weight(side_key_w).special(),
                        Key::text(",").with_weight(1.0),
                        Key::new("Space", KeyAction::Space).with_weight(p_space_w),
                        Key::text(".").with_weight(1.0),
                        Key::new("⏎", KeyAction::Enter).with_weight(side_key_w).special(),
                    ],
                },
            ],
        }
        .with_top_bar(suggestions)
    }
}

fn clipboard_label(text: &str) -> String {
    let mut label = text.to_string();
    if label.chars().count() > 26 {
        let cut: String = label.chars().take(26).collect();
        label = format!("{cut}…");
    }
    label
}