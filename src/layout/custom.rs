use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

use crate::layout::key::{Key, KeyAction, LayerId, LayoutMode};
use crate::layout::layouts::{KeyboardLayout, KeyboardRow};

/// Key definition in the JSON matrix.
/// Can be either a simple string (e.g. `"q"` or `"Enter"`) or an object with options:
/// `{"key": "q", "sub": "1", "width": 1.0, "action": "text"}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KeyDef {
    Simple(String),
    Detailed(KeyObject),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyObject {
    /// Primary key label (e.g. "q", "Enter", "⇧", "Space")
    pub key: String,
    /// Subscript / secondary label for dual keys (e.g. "1", "@")
    #[serde(default)]
    pub sub: Option<String>,
    /// Relative width weight (default 1.0)
    #[serde(default = "default_key_weight")]
    pub width: f32,
    /// Action string: "text", "shift", "backspace", "enter", "space", "tab", "escape",
    /// "ctrl", "alt", "win", "home", "end", "left", "right", "up", "down",
    /// "symbols", "symbols2", "lower", "clipboard", "hide", "none"
    #[serde(default)]
    pub action: Option<String>,
    /// Optional explicit text payload if different from label
    #[serde(default)]
    pub text: Option<String>,
    /// Whether this key should be styled as a special/accent key
    #[serde(default)]
    pub special: Option<bool>,
    /// Custom spacing / gap to the right of this key in pixels
    #[serde(default)]
    pub margin_right: Option<f32>,
    /// Custom spacing / gap to the left of this key in pixels
    #[serde(default)]
    pub margin_left: Option<f32>,
    /// Alias for margin_right (gap between this key and the next)
    #[serde(default)]
    pub gap: Option<f32>,
    /// Whether this key has borders (set false for borderless)
    #[serde(default)]
    pub borders: Option<bool>,
    /// Optional key border width in pixels
    #[serde(default)]
    pub border_width: Option<f32>,
    /// Primary font size/scale (e.g. 1.0, 1.5, 2.0)
    #[serde(default)]
    pub font_size: Option<f32>,
    /// Secondary/subscript font size/scale (e.g. 1.0, 1.5)
    #[serde(default)]
    pub secondary_font_size: Option<f32>,
    /// Optional explicit height for this key in pixels
    #[serde(default)]
    pub height: Option<f32>,
    /// Optional explicit width for this key in pixels (overrides relative width weight)
    #[serde(default)]
    pub width_px: Option<f32>,
}

fn default_key_weight() -> f32 {
    1.0
}

/// A row definition can be either a simple array of keys:
/// `[ "q", "w", "e", ... ]`
/// or an object with keys and custom row spacing:
/// `{"keys": [ "q", "w", ... ], "margin_bottom": 6.0}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RowDef {
    Simple(Vec<KeyDef>),
    Detailed(RowObject),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowObject {
    pub keys: Vec<KeyDef>,
    /// Custom vertical spacing after this row in pixels
    #[serde(default)]
    pub margin_bottom: Option<f32>,
}

/// A row of keys in the matrix
pub type MatrixRow = RowDef;

/// Complete custom layout definition parsed from JSON
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomLayoutFile {
    /// Human-readable layout name (e.g. "Mobile Layout")
    #[serde(default = "default_layout_name")]
    pub name: String,
    /// Target mode hint: "mobile" or "desktop" (default: "mobile")
    #[serde(default = "default_layout_mode")]
    pub mode: String,
    /// Optional explicit layout width in pixels (e.g. 392 for phone screen factor).
    /// If omitted or None, the layout spans full screen width.
    #[serde(default)]
    pub width: Option<u32>,
    /// Optional explicit layout height in pixels (e.g. 290).
    /// If omitted or None, uses the height configured in config.toml.
    #[serde(default)]
    pub height: Option<u32>,
    /// Optional bottom margin in pixels (e.g. 16 for floating dock aesthetic).
    #[serde(default)]
    pub margin_bottom: Option<i32>,
    /// Global key horizontal spacing override in pixels (e.g. 4.0 or 6.0)
    #[serde(default)]
    pub key_spacing: Option<f32>,
    /// Global row vertical spacing override in pixels (e.g. 4.0 or 6.0)
    #[serde(default)]
    pub row_spacing: Option<f32>,
    /// Layout horizontal padding in pixels (e.g. 4.0)
    #[serde(default)]
    pub padding_x: Option<f32>,
    /// Layout vertical padding in pixels (e.g. 4.0)
    #[serde(default)]
    pub padding_y: Option<f32>,
    /// Whether keys have borders (set false for borderless keys)
    #[serde(default)]
    pub borders: Option<bool>,
    /// Global border width override in pixels
    #[serde(default)]
    pub border_width: Option<f32>,
    /// Global primary font size/scale (e.g. 1.0, 1.5, 2.0)
    #[serde(default)]
    pub font_size: Option<f32>,
    /// Global secondary font size/scale (e.g. 1.0, 1.5)
    #[serde(default)]
    pub secondary_font_size: Option<f32>,
    /// Global key height override in pixels
    #[serde(default)]
    pub key_height: Option<f32>,
    /// Global key width override in pixels
    #[serde(default)]
    pub key_width: Option<f32>,
    /// Layers map: "lower", "upper", "symbols", "symbols2"
    #[serde(default)]
    pub layers: HashMap<String, Vec<MatrixRow>>,
}

fn default_layout_name() -> String {
    "Custom Layout".to_string()
}

fn default_layout_mode() -> String {
    "mobile".to_string()
}

impl CustomLayoutFile {
    /// Locate and load a layout from either:
    /// 1. An explicit file path: `/path/to/layout.json` or `./my_layout.json`
    /// 2. A layout name in `~/.config/hyprosk/layouts/<name>.json`
    pub fn load_from_name_or_path(name_or_path: &str) -> anyhow::Result<Self> {
        let path = Self::resolve_layout_path(name_or_path);
        tracing::info!("Loading custom layout from {:?}", path);
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read layout file at {:?}: {}", path, e))?;
        let parsed: Self = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse layout JSON in {:?}: {}", path, e))?;
        Ok(parsed)
    }

    /// Resolve layout name or path to a filesystem `PathBuf`.
    pub fn resolve_layout_path(name_or_path: &str) -> PathBuf {
        let p = Path::new(name_or_path);
        if p.is_absolute() || name_or_path.starts_with("./") || name_or_path.starts_with("../") {
            return p.to_path_buf();
        }

        let base_dir = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg).join("hyprosk/layouts")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config/hyprosk/layouts")
        } else {
            PathBuf::from("layouts")
        };

        let filename = if name_or_path.ends_with(".json") {
            name_or_path.to_string()
        } else {
            format!("{}.json", name_or_path)
        };

        base_dir.join(filename)
    }

    /// Convert custom layout layer into a runtime `KeyboardLayout`
    pub fn to_keyboard_layout(
        &self,
        layer_id: LayerId,
        suggestions: &[String],
        caps_lock: bool,
    ) -> KeyboardLayout {
        let layer_key = match layer_id {
            LayerId::Lower => "lower",
            LayerId::Upper => "upper",
            LayerId::Symbols => "symbols",
            LayerId::Symbols2 => "symbols2",
        };

        // If requested layer is not present, fall back to "lower" or fallback to built-in layout
        let matrix = self
            .layers
            .get(layer_key)
            .or_else(|| self.layers.get("lower"));

        let mode = LayoutMode::parse_mode(&self.mode).unwrap_or(LayoutMode::Mobile);

        let rows = if let Some(matrix_rows) = matrix {
            let is_upper = layer_id == LayerId::Upper;
            let auto_upper = is_upper && !self.layers.contains_key("upper");

            matrix_rows
                .iter()
                .map(|row_def| match row_def {
                    RowDef::Simple(defs) => {
                        let keys: Vec<Key> = defs
                            .iter()
                            .map(|def| convert_key_def(def, auto_upper, caps_lock))
                            .collect();
                        KeyboardRow::new(keys)
                    }
                    RowDef::Detailed(obj) => {
                        let keys: Vec<Key> = obj
                            .keys
                            .iter()
                            .map(|def| convert_key_def(def, auto_upper, caps_lock))
                            .collect();
                        let mut row = KeyboardRow::new(keys);
                        row.margin_bottom = obj.margin_bottom;
                        row
                    }
                })
                .collect()
        } else {
            // Fall back to built-in layout for this layer
            return KeyboardLayout::get_layout_for_mode(layer_id, suggestions, caps_lock, mode);
        };

        let mut layout = KeyboardLayout {
            id: layer_id,
            rows,
            key_spacing: self.key_spacing,
            row_spacing: self.row_spacing,
            padding_x: self.padding_x,
            padding_y: self.padding_y,
            borders: self.borders,
            border_width: self.border_width,
            font_size: self.font_size,
            secondary_font_size: self.secondary_font_size,
            key_height: self.key_height,
            key_width: self.key_width,
        };

        // Always prepend the standard top action/suggestion bar
        layout.rows.insert(0, KeyboardLayout::make_top_bar(suggestions));
        layout
    }
}

fn convert_key_def(def: &KeyDef, auto_upper: bool, caps_lock: bool) -> Key {
    match def {
        KeyDef::Simple(s) => {
            let label = if auto_upper && s.chars().count() == 1 {
                s.to_uppercase()
            } else {
                s.clone()
            };
            let action = infer_action(&label, None, None);
            let is_special = is_default_special(&label, &action);
            let mut key = Key::new(label, action);
            key.is_special = is_special;
            key
        }
        KeyDef::Detailed(obj) => {
            let mut label = if auto_upper && obj.key.chars().count() == 1 {
                obj.key.to_uppercase()
            } else {
                obj.key.clone()
            };

            let action = infer_action(&label, obj.action.as_deref(), obj.text.as_deref());

            // If shift key and caps lock active, show caps icon
            if matches!(action, KeyAction::Shift) && caps_lock {
                label = "⇪".to_string();
            }

            let mut key = Key::new(label, action);
            key.width_weight = obj.width;
            if let Some(sub) = &obj.sub {
                key.secondary_label = Some(sub.clone());
            }
            key.is_special = obj.special.unwrap_or_else(|| is_default_special(&key.label, &key.action));
            key.margin_left = obj.margin_left;
            key.margin_right = obj.margin_right.or(obj.gap);
            key.borders = obj.borders;
            key.border_width = obj.border_width;
            key.font_size = obj.font_size;
            key.secondary_font_size = obj.secondary_font_size;
            key.height = obj.height;
            key.width_px = obj.width_px;
            key
        }
    }
}

fn infer_action(label: &str, action_str: Option<&str>, text_override: Option<&str>) -> KeyAction {
    if let Some(act) = action_str {
        match act.trim().to_lowercase().as_str() {
            "shift" => return KeyAction::Shift,
            "backspace" | "bksp" | "delete" | "del" => return KeyAction::Backspace,
            "enter" | "return" => return KeyAction::Enter,
            "space" => return KeyAction::Space,
            "tab" => return KeyAction::Tab,
            "escape" | "esc" => return KeyAction::Escape,
            "ctrl" => return KeyAction::Ctrl,
            "alt" => return KeyAction::Alt,
            "win" | "super" | "meta" => return KeyAction::Win,
            "home" => return KeyAction::Home,
            "end" => return KeyAction::End,
            "left" | "arrow_left" => return KeyAction::ArrowLeft,
            "right" | "arrow_right" => return KeyAction::ArrowRight,
            "up" | "arrow_up" => return KeyAction::ArrowUp,
            "down" | "arrow_down" => return KeyAction::ArrowDown,
            "symbols" | "sym" | "123" | "?123" => return KeyAction::SwitchLayer(LayerId::Symbols),
            "symbols2" | "sym2" | "=<" | "~[<" => return KeyAction::SwitchLayer(LayerId::Symbols2),
            "lower" | "abc" | "letters" => return KeyAction::SwitchLayer(LayerId::Lower),
            "upper" => return KeyAction::SwitchLayer(LayerId::Upper),
            "clipboard" | "clip" => return KeyAction::Clipboard,
            "hide" | "dismiss" => return KeyAction::Hide,
            "none" | "null" => return KeyAction::None,
            "text" => {
                let t = text_override.unwrap_or(label);
                return KeyAction::Text(t.to_string());
            }
            _ => {}
        }
    }

    if let Some(txt) = text_override {
        return KeyAction::Text(txt.to_string());
    }

    // Infer from standard labels
    match label {
        "⇧" | "Shift" | "⇪" => KeyAction::Shift,
        "⌫" | "Backspace" | "Bksp" | "Del" => KeyAction::Backspace,
        "⏎" | "Enter" | "Return" => KeyAction::Enter,
        "Space" | "␣" => KeyAction::Space,
        "Tab" => KeyAction::Tab,
        "Esc" | "Escape" => KeyAction::Escape,
        "Ctrl" => KeyAction::Ctrl,
        "Alt" => KeyAction::Alt,
        "⊞" | "Win" | "Super" => KeyAction::Win,
        "?123" | "&123" | "123" => KeyAction::SwitchLayer(LayerId::Symbols),
        "=\\<" | "=<" | "~[<" => KeyAction::SwitchLayer(LayerId::Symbols2),
        "abc" | "ABC" => KeyAction::SwitchLayer(LayerId::Lower),
        "◀" => KeyAction::ArrowLeft,
        "▶" => KeyAction::ArrowRight,
        "▲" => KeyAction::ArrowUp,
        "▼" => KeyAction::ArrowDown,
        "Home" => KeyAction::Home,
        "End" => KeyAction::End,
        "🎤" => KeyAction::None,
        other => KeyAction::Text(other.to_string()),
    }
}

fn is_default_special(label: &str, action: &KeyAction) -> bool {
    match action {
        KeyAction::Shift
        | KeyAction::Backspace
        | KeyAction::Enter
        | KeyAction::Tab
        | KeyAction::Escape
        | KeyAction::Ctrl
        | KeyAction::Alt
        | KeyAction::Win
        | KeyAction::SwitchLayer(_)
        | KeyAction::ArrowLeft
        | KeyAction::ArrowRight
        | KeyAction::ArrowUp
        | KeyAction::ArrowDown
        | KeyAction::Home
        | KeyAction::End
        | KeyAction::Hide
        | KeyAction::Clipboard => true,
        KeyAction::None => label == "🎤" || label == "gear" || label == "palette",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_custom_json() {
        let json_str = r#"{
            "name": "Test Layout",
            "mode": "mobile",
            "layers": {
                "lower": [
                    [{"key": "q", "sub": "1"}, "w", "e", "r", "t", "y", "u", "i", "o", "p"],
                    ["a", "s", "d", "f", "g", "h", "j", "k", "l"],
                    [{"key": "⇧", "action": "shift", "width": 1.35}, "z", "x", "c", "v", "b", "n", "m", {"key": "⌫", "action": "backspace", "width": 1.35}],
                    [{"key": "?123", "action": "symbols", "width": 1.35}, ",", {"key": "Space", "action": "space", "width": 4.9}, ".", {"key": "⏎", "action": "enter", "width": 1.35}]
                ]
            }
        }"#;

        let parsed: CustomLayoutFile = serde_json::from_str(json_str).expect("parse json");
        assert_eq!(parsed.name, "Test Layout");
        assert_eq!(parsed.mode, "mobile");
        assert!(parsed.layers.contains_key("lower"));

        let k_layout = parsed.to_keyboard_layout(LayerId::Lower, &[], false);
        // Top action row + 4 keyboard rows = 5 rows
        assert_eq!(k_layout.rows.len(), 5);
        // Row 1 (letters row 1) has 10 keys
        assert_eq!(k_layout.rows[1].keys.len(), 10);
        assert_eq!(k_layout.rows[1].keys[0].label, "q");
        assert_eq!(k_layout.rows[1].keys[0].secondary_label.as_deref(), Some("1"));
        // Row 2 has 9 keys
        assert_eq!(k_layout.rows[2].keys.len(), 9);
        // Spacebar in row 4 has width 4.9 and action Space
        assert_eq!(k_layout.rows[4].keys[2].action, KeyAction::Space);
        assert!((k_layout.rows[4].keys[2].width_weight - 4.9).abs() < 0.01);
    }

    #[test]
    fn test_custom_spacing_and_margins() {
        let json_str = r#"{
            "name": "Custom Spacing Layout",
            "width": 392,
            "height": 290,
            "margin_bottom": 16,
            "key_spacing": 4.5,
            "row_spacing": 5.0,
            "padding_x": 6.0,
            "padding_y": 7.0,
            "layers": {
                "lower": [
                    {
                        "margin_bottom": 8.0,
                        "keys": [
                            {"key": "a", "margin_right": 2.0},
                            {"key": "b", "margin_left": 1.0, "gap": 3.0}
                        ]
                    }
                ]
            }
        }"#;

        let parsed: CustomLayoutFile = serde_json::from_str(json_str).expect("parse json");
        assert_eq!(parsed.width, Some(392));
        assert_eq!(parsed.height, Some(290));
        assert_eq!(parsed.margin_bottom, Some(16));
        assert_eq!(parsed.key_spacing, Some(4.5));
        assert_eq!(parsed.row_spacing, Some(5.0));
        assert_eq!(parsed.padding_x, Some(6.0));
        assert_eq!(parsed.padding_y, Some(7.0));

        let k_layout = parsed.to_keyboard_layout(LayerId::Lower, &[], false);
        assert_eq!(k_layout.key_spacing, Some(4.5));
        assert_eq!(k_layout.row_spacing, Some(5.0));
        assert_eq!(k_layout.padding_x, Some(6.0));
        assert_eq!(k_layout.padding_y, Some(7.0));

        // Row 1 is our custom row (index 0 is the prepended top bar)
        let custom_row = &k_layout.rows[1];
        assert_eq!(custom_row.margin_bottom, Some(8.0));
        assert_eq!(custom_row.keys.len(), 2);
        assert_eq!(custom_row.keys[0].margin_right, Some(2.0));
        assert_eq!(custom_row.keys[1].margin_left, Some(1.0));
        assert_eq!(custom_row.keys[1].margin_right, Some(3.0));
    }

    #[test]
    fn test_custom_styling_and_dimensions() {
        let json_str = r#"{
            "name": "Borderless Layout",
            "borders": false,
            "border_width": 2.0,
            "font_size": 2.0,
            "secondary_font_size": 1.0,
            "key_height": 55.0,
            "key_width": 36.0,
            "layers": {
                "lower": [
                    [
                        {"key": "a", "borders": true, "height": 60.0, "width_px": 40.0, "font_size": 3.0},
                        {"key": "b", "borders": false}
                    ]
                ]
            }
        }"#;

        let parsed: CustomLayoutFile = serde_json::from_str(json_str).expect("parse json");
        assert_eq!(parsed.borders, Some(false));
        assert_eq!(parsed.border_width, Some(2.0));
        assert_eq!(parsed.font_size, Some(2.0));
        assert_eq!(parsed.secondary_font_size, Some(1.0));
        assert_eq!(parsed.key_height, Some(55.0));
        assert_eq!(parsed.key_width, Some(36.0));

        let k_layout = parsed.to_keyboard_layout(LayerId::Lower, &[], false);
        assert_eq!(k_layout.borders, Some(false));
        assert_eq!(k_layout.border_width, Some(2.0));
        assert_eq!(k_layout.font_size, Some(2.0));
        assert_eq!(k_layout.secondary_font_size, Some(1.0));
        assert_eq!(k_layout.key_height, Some(55.0));
        assert_eq!(k_layout.key_width, Some(36.0));

        let k0 = &k_layout.rows[1].keys[0];
        assert_eq!(k0.borders, Some(true));
        assert_eq!(k0.height, Some(60.0));
        assert_eq!(k0.width_px, Some(40.0));
        assert_eq!(k0.font_size, Some(3.0));

        let k1 = &k_layout.rows[1].keys[1];
        assert_eq!(k1.borders, Some(false));
    }

    #[test]
    fn test_mobile_symbols2_row1() {
        let path = "/home/CheeseGuru/.config/hyprosk/layouts/mobile.json";
        if let Ok(layout_file) = CustomLayoutFile::load_from_name_or_path(path) {
            let k_layout = layout_file.to_keyboard_layout(LayerId::Symbols2, &[], false);
            let r1 = &k_layout.rows[1];
            assert_eq!(r1.keys.len(), 10);
            let labels: Vec<&str> = r1.keys.iter().map(|k| k.label.as_str()).collect();
            assert_eq!(labels, vec!["~", "`", "|", "•", "√", "π", "÷", "×", "¶", "∆"]);
        }
    }
}
