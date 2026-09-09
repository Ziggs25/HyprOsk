use crate::layout::key::{Key, KeyAction, LayerId, LayoutMode};

#[derive(Debug, Clone, PartialEq)]
pub struct KeyboardRow {
    pub keys: Vec<Key>,
    /// Optional custom spacing after this row in pixels
    pub margin_bottom: Option<f32>,
}

impl KeyboardRow {
    pub fn new(keys: Vec<Key>) -> Self {
        Self {
            keys,
            margin_bottom: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyboardLayout {
    pub rows: Vec<KeyboardRow>,
    pub id: LayerId,
    /// Optional default key-to-key horizontal spacing override in pixels
    pub key_spacing: Option<f32>,
    /// Optional default row-to-row vertical spacing override in pixels
    pub row_spacing: Option<f32>,
    /// Optional horizontal padding override in pixels
    pub padding_x: Option<f32>,
    /// Optional vertical padding override in pixels
    pub padding_y: Option<f32>,
    /// Whether keys have borders (set false for borderless keys)
    pub borders: Option<bool>,
    /// Border width override in pixels
    pub border_width: Option<f32>,
    /// Global primary font size/scale (e.g. 1.0, 1.5, 2.0)
    pub font_size: Option<f32>,
    /// Global secondary font size/scale (e.g. 1.0, 1.5)
    pub secondary_font_size: Option<f32>,
    /// Global explicit key height override in pixels
    pub key_height: Option<f32>,
    /// Global explicit key width override in pixels
    pub key_width: Option<f32>,
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

    pub fn get_layout_for_mode(id: LayerId, suggestions: &[String], caps_lock: bool, _mode: LayoutMode) -> Self {
        Self::get_layout_with_caps(id, suggestions, caps_lock)
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
        // Left: Clipboard
        keys.push(Key::new("", KeyAction::Clipboard).with_weight(action_weight).special());

        // Center: 3 suggestions
        let s0 = suggestions.first().map(|s| s.as_str()).unwrap_or("");
        let s1 = suggestions.get(1).map(|s| s.as_str()).unwrap_or("");
        let s2 = suggestions.get(2).map(|s| s.as_str()).unwrap_or("");

        keys.push(Key::suggestion(0, s0).with_weight(1.0));
        keys.push(Key::suggestion(1, s1).with_weight(1.3));
        keys.push(Key::suggestion(2, s2).with_weight(1.0));

        // Right: Hide
        keys.push(Key::new("", KeyAction::Hide).with_weight(action_weight).special());
        KeyboardRow::new(keys)
    }

    /// Helper to create a new layout with defaults
    pub fn new(id: LayerId, rows: Vec<KeyboardRow>) -> Self {
        Self {
            rows,
            id,
            key_spacing: None,
            row_spacing: None,
            padding_x: None,
            padding_y: None,
            borders: None,
            border_width: None,
            font_size: None,
            secondary_font_size: None,
            key_height: None,
            key_width: None,
        }
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

        Self::new(
            id,
            vec![
                KeyboardRow::new(vec![
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
                ]),
                KeyboardRow::new(vec![
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
                ]),
                KeyboardRow::new(vec![
                    Key::new(shift_label, KeyAction::Shift).with_weight(SHIFT).special(),
                    dual('z', '*'),
                    dual('x', '"'),
                    dual('c', '\''),
                    dual('v', ':'),
                    dual('b', ';'),
                    dual('n', '!'),
                    dual('m', '?'),
                    Key::new(shift_label, KeyAction::Shift).with_weight(SHIFT).special(),
                ]),
                KeyboardRow::new(vec![
                    Key::new("&123", KeyAction::SwitchLayer(LayerId::Symbols)).with_weight(TOGGLE).special(),
                    Key::new("Ctrl", KeyAction::Ctrl).with_weight(MODIFIER).special(),
                    Key::new("⊞", KeyAction::Win).with_weight(MODIFIER).special(),
                    Key::new("Alt", KeyAction::Alt).with_weight(MODIFIER).special(),
                    Key::new("Space", KeyAction::Space).with_weight(SPACE),
                    Key::new("🎤", KeyAction::None).with_weight(MODIFIER).special(),
                    Key::new("◀", KeyAction::ArrowLeft).with_weight(MODIFIER).special(),
                    Key::new("▶", KeyAction::ArrowRight).with_weight(MODIFIER).special(),
                ]),
            ],
        )
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
        Self::new(
            LayerId::Symbols,
            vec![
                KeyboardRow::new(vec![
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
                ]),
                KeyboardRow::new(vec![
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
                ]),
                KeyboardRow::new(vec![
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
                ]),
                KeyboardRow::new(vec![
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
                ]),
            ],
        )
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
        Self::new(
            LayerId::Symbols2,
            vec![
                KeyboardRow::new(vec![
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
                ]),
                KeyboardRow::new(vec![
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
                ]),
                KeyboardRow::new(vec![
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
                ]),
                KeyboardRow::new(vec![
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
                ]),
            ],
        )
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

        let mut rows = vec![KeyboardRow::new(top)];
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
            rows.push(KeyboardRow::new(row_keys));
        }

        Self::new(LayerId::Lower, rows).with_top_bar(suggestions)
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