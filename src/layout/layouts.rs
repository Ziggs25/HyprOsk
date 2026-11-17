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
    /// Whether the hold-to-preview popup for secondary labels is shown.
    /// `Some(false)` disables it; default (None) means enabled.
    pub preview_enabled: Option<bool>,
    /// Explicit hold-preview popup height in pixels.
    /// If unset, defaults to 1.65x key height (min 115px).
    pub preview_height: Option<f32>,
    /// Hold-preview popup label font size in pixels (default 34).
    pub preview_font_size: Option<f32>,
    /// Optional color palette overrides
    pub colors: Option<crate::layout::custom::LayoutColors>,
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
            preview_enabled: None,
            preview_height: None,
            preview_font_size: None,
            colors: None,
        }
    }

    /// Clipboard history view replacing the key rows (`wireframe-2.md` View 1).
    ///
    /// Row 1: ◀ Back · header
    /// Rows 2-3: up to 8 history entries presented as suggestion pills;
    /// tapping an entry pastes it.
    /// Last row: ABC (back to letters) · Space · ⌫ (standard key row).
    ///
    /// Clipboard cards reflow between one and four columns as the window width changes.
    pub const CLIPBOARD_MAX_COLUMNS: usize = 4;
    pub const CLIPBOARD_PANEL_PADDING: f32 = 8.0;
    pub const CLIPBOARD_SIDE_INSET: f32 = 12.0;
    pub const CLIPBOARD_CARD_GAP: f32 = 12.0;
    pub const CLIPBOARD_MIN_CARD_WIDTH: f32 = 210.0;
    pub const CLIPBOARD_MAX_CARD_WIDTH: f32 = 520.0;

    pub fn clipboard_grid_padding(padding_x: f32) -> f32 {
        padding_x.max(Self::CLIPBOARD_PANEL_PADDING) + Self::CLIPBOARD_SIDE_INSET
    }

    pub fn clipboard_columns_for_width(total_width: u32, padding_x: f32) -> usize {
        let grid_padding_x = Self::clipboard_grid_padding(padding_x);
        let available_width = (total_width as f32 - 2.0 * grid_padding_x).max(0.0);
        let columns = ((available_width + Self::CLIPBOARD_CARD_GAP)
            / (Self::CLIPBOARD_MIN_CARD_WIDTH + Self::CLIPBOARD_CARD_GAP))
            .floor() as usize;
        columns.clamp(1, Self::CLIPBOARD_MAX_COLUMNS)
    }

    #[allow(clippy::needless_lifetimes)]
    pub fn clipboard(
        history: &[String],
        suggestions: &[String],
        width: u32,
        padding_x: Option<f32>,
    ) -> Self {
        let padding_x = padding_x.unwrap_or(if width < 600 { 3.5 } else { 8.0 });
        let columns = Self::clipboard_columns_for_width(width, padding_x);
        // Row 0: Top action bar.
        // Key 0: ◀ Back (KeyAction::Clipboard)
        // Middle: suggestions (or "Clipboard History" title if none)
        // Right: KeyAction::Hide (closes keyboard)
        let mut top_keys = Vec::new();
        let action_weight = 0.35;
        top_keys.push(Key::new("◀ Back", KeyAction::Clipboard).with_weight(action_weight).special().clipboard());

        let s0 = suggestions.first().map(|s| s.as_str()).unwrap_or("Clipboard History");
        let s1 = suggestions.get(1).map(|s| s.as_str()).unwrap_or("");
        let s2 = suggestions.get(2).map(|s| s.as_str()).unwrap_or("");

        top_keys.push(Key::suggestion(0, s0).with_weight(1.0));
        top_keys.push(Key::suggestion(1, s1).with_weight(1.3));
        top_keys.push(Key::suggestion(2, s2).with_weight(1.0));

        top_keys.push(Key::new("", KeyAction::Hide).with_weight(action_weight).special());

        let mut rows = vec![KeyboardRow::new(top_keys)];
        let mut idx = 0usize;
        for chunk in history.chunks(columns) {
            let row_keys: Vec<Key> = chunk
                .iter()
                .map(|text| {
                    let label = clipboard_label(text);
                    let mut key = Key::new(label, KeyAction::ClipboardItem(idx))
                        .special()
                        .clipboard()
                        .with_weight(1.0)
                        .with_height(168.0);
                    if idx == 0 {
                        key = key.pinned();
                    }
                    idx += 1;
                    key
                })
                .collect();
            let mut row = KeyboardRow::new(row_keys);
            row.margin_bottom = Some(16.0);
            rows.push(row);
        }

        // Bottom row: ABC (back to letters) · Space · Backspace.
        rows.push(clipboard_bottom_row());

        Self::new(LayerId::Lower, rows)
    }
}

/// Bottom row for the clipboard view (HeliBoard/Gboard-style).
///
/// ABC returns to the letter layer, Space types a space, ⌫ deletes.
/// These are standard keys (NOT `is_clipboard` chips) so
/// `RenderEngine::calculate_key_rects` gives them the normal key-row
/// height instead of the compact clipboard-card geometry.
fn clipboard_bottom_row() -> KeyboardRow {
    KeyboardRow::new(vec![
        Key::new("ABC", KeyAction::SwitchLayer(LayerId::Lower))
            .with_weight(1.5)
            .special(),
        Key::new("Space", KeyAction::Space).with_weight(5.0),
        Key::new("⌫", KeyAction::Backspace)
            .with_weight(1.5)
            .special(),
    ])
}

fn clipboard_label(text: &str) -> String {
    // Collapse whitespace/newlines so multi-line copies preview on one card,
    // then show up to 180 characters in the larger square-ish preview.
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.chars().count() > 180 {
        let cut: String = trimmed.chars().take(180).collect();
        format!("{cut}…")
    } else {
        trimmed.to_string()
    }
}