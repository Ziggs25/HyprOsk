#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerId {
    Lower,
    Upper,
    Numbers,
    Symbols,
    Nav,
    Emoji,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KeyAction {
    Text(String),
    Keycode(u32),
    Backspace,
    Enter,
    Space,
    SpaceSwipe,
    Tab,
    Escape,
    Shift,
    CapsLock,
    SwitchLayer(LayerId),
    Suggestion(usize),
    Hide,
    Copy,
    Paste,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
}

#[derive(Debug, Clone)]
pub struct Key {
    pub label: String,
    pub secondary_label: Option<String>,
    pub action: KeyAction,
    pub width_weight: f32,
    pub is_special: bool,
    pub is_pressed: bool,
    pub is_locked: bool,
    pub is_suggestion: bool,
}

impl Key {
    pub fn new(label: impl Into<String>, action: KeyAction) -> Self {
        Self {
            label: label.into(),
            secondary_label: None,
            action,
            width_weight: 1.0,
            is_special: false,
            is_pressed: false,
            is_locked: false,
            is_suggestion: false,
        }
    }

    pub fn text(label: impl Into<String>) -> Self {
        let label_str = label.into();
        Self {
            action: KeyAction::Text(label_str.clone()),
            label: label_str,
            secondary_label: None,
            width_weight: 1.0,
            is_special: false,
            is_pressed: false,
            is_locked: false,
            is_suggestion: false,
        }
    }

    pub fn suggestion(index: usize, label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            secondary_label: None,
            action: KeyAction::Suggestion(index),
            width_weight: 1.0,
            is_special: false,
            is_pressed: false,
            is_locked: false,
            is_suggestion: true,
        }
    }

    pub fn with_secondary(mut self, sec: impl Into<String>) -> Self {
        self.secondary_label = Some(sec.into());
        self
    }

    pub fn with_weight(mut self, weight: f32) -> Self {
        self.width_weight = weight;
        self
    }

    pub fn special(mut self) -> Self {
        self.is_special = true;
        self
    }
}
