#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerId {
    /// QWERTY letters (lowercase)
    Lower,
    /// QWERTY letters (uppercase, via Shift)
    Upper,
    /// Symbols page 1: numbers & primary symbols (wireframe-2 View 3)
    Symbols,
    /// Symbols page 2: extended symbols & currency (wireframe-2 View 4)
    Symbols2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LayoutMode {
    /// Desktop layout (Windows 11 style with Esc, Tab, Ctrl, Alt, Win, Arrows)
    #[default]
    Desktop,
    /// Mobile layout (HeliBoard / Gboard style with clean mobile ergonomics)
    Mobile,
}

impl LayoutMode {
    pub fn is_mobile(&self) -> bool {
        matches!(self, LayoutMode::Mobile)
    }

    pub fn parse_mode(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "mobile" | "phone" | "portrait" | "gboard" | "heliboard" => Some(LayoutMode::Mobile),
            "desktop" | "landscape" | "win11" | "full" => Some(LayoutMode::Desktop),
            _ => None,
        }
    }
}

impl std::str::FromStr for LayoutMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_mode(s).ok_or_else(|| format!("Unknown layout mode '{}'. Expected 'desktop' or 'mobile'.", s))
    }
}

impl std::fmt::Display for LayoutMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutMode::Desktop => write!(f, "desktop"),
            LayoutMode::Mobile => write!(f, "mobile"),
        }
    }
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
    /// Toggle the clipboard history view (replaces the key rows).
    Clipboard,
    /// Paste the clipboard history entry at the given index.
    ClipboardItem(usize),
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    /// Momentary Ctrl modifier
    Ctrl,
    /// Momentary Alt modifier
    Alt,
    /// Momentary Win / Super modifier
    Win,
    /// Home navigation key
    Home,
    /// End navigation key
    End,
    /// Visual-only key (e.g. Mic): renders but performs no action
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    pub label: String,
    pub secondary_label: Option<String>,
    pub action: KeyAction,
    pub width_weight: f32,
    pub is_special: bool,
    pub is_pressed: bool,
    pub is_locked: bool,
    pub is_suggestion: bool,
    pub is_clipboard: bool,
    pub is_pinned: bool,
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
            is_clipboard: false,
            is_pinned: false,
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
            is_clipboard: false,
            is_pinned: false,
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
            is_clipboard: false,
            is_pinned: false,
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

    pub fn clipboard(mut self) -> Self {
        self.is_clipboard = true;
        self
    }

    pub fn pinned(mut self) -> Self {
        self.is_pinned = true;
        self
    }
}
