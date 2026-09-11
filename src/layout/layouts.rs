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
    /// Whether to enable semi-transparent background for Hyprland compositor blurring
    pub blur: Option<bool>,
    /// Optional background opacity override (e.g. 0.60)
    pub opacity: Option<f32>,
    /// Optional key background opacity override (e.g. 0.50)
    pub key_opacity: Option<f32>,
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

    pub fn get_layout_for_mode(id: LayerId, suggestions: &[String], caps_lock: bool, mode: LayoutMode) -> Self {
        let custom = crate::layout::custom::CustomLayoutFile::get_default_layout(mode);
        custom.to_keyboard_layout(id, suggestions, caps_lock)
    }

    pub fn get_layout_with_caps(id: LayerId, suggestions: &[String], caps_lock: bool) -> Self {
        Self::get_layout_for_mode(id, suggestions, caps_lock, LayoutMode::Desktop)
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

    /// Prepend the standard top/action bar with suggestions to this layout's rows.
    pub fn with_top_bar(mut self, suggestions: &[String]) -> Self {
        self.rows.insert(0, Self::make_top_bar(suggestions));
        self
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
            blur: None,
            opacity: None,
            key_opacity: None,
        }
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