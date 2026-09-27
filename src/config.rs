use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub behavior: BehaviorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Height of keyboard in pixels (or percentage if negative/relative)
    pub height: u32,
    /// Margin from bottom edge in pixels
    pub margin_bottom: i32,
    /// Margin left and right in pixels
    pub margin_horizontal: i32,
    /// Corner radius of the keyboard window
    pub corner_radius: f32,
    /// Layer shell exclusivity (whether to push up windows)
    pub exclusive_zone: bool,
    /// Active theme name (catppuccin, tokyo-night, oled, nord)
    pub theme_name: String,
    /// Keyboard layout style: "desktop" (default Windows 11 style) or "mobile" (HeliBoard / Gboard style)
    #[serde(default = "default_layout_mode")]
    pub layout_mode: String,
    /// Path or name of custom JSON layout file (looked up in ~/.config/hyprosk/layouts/<name>.json)
    #[serde(default)]
    pub layout_file: Option<String>,
}

fn default_layout_mode() -> String {
    "desktop".to_string()
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            height: 420,
            margin_bottom: 0,
            margin_horizontal: 0,
            corner_radius: 0.0,
            exclusive_zone: true,
            theme_name: "oled".to_string(),
            layout_mode: "desktop".to_string(),
            layout_file: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub background: String,
    pub key_background: String,
    pub key_pressed: String,
    pub key_special: String,
    pub text_color: String,
    pub text_special: String,
    pub accent_color: String,
    pub border_color: String,
    pub border_width: f32,
    pub key_radius: f32,
    pub key_spacing: f32,
    pub opacity: f32,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            background: "#000000f5".to_string(),     // Deep solid black base
            key_background: "#181818".to_string(), // Dark neutral key surface
            key_pressed: "#0096ff".to_string(),    // Accent on press
            key_special: "#262626".to_string(),    // Subtle dark for functional keys
            text_color: "#f0f0f0".to_string(),     // Crisp light legend text
            text_special: "#ffd700".to_string(),   // Gold/warm accent for special text
            accent_color: "#00c8ff".to_string(),   // Bright accent
            border_color: "#3c3c3c78".to_string(), // Muted border
            border_width: 1.0,
            key_radius: 8.0,
            key_spacing: 6.0,
            opacity: 0.95,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorConfig {
    /// Automatically show keyboard when text input field is focused (zwp_input_method_v2)
    pub auto_show: bool,
    /// Automatically hide keyboard when text input field loses focus
    pub auto_hide: bool,
    /// Automatically hide keyboard on fullscreen windows
    pub hide_on_fullscreen: bool,
    /// Only auto-show when no physical (folio) keyboard is attached; manual
    /// `hyprosk show` / `toggle` always still works
    pub folio_mode: bool,
    /// Only auto-show when the last input that triggered focus was touch.
    /// Mirrors GNOME `lastDeviceIsTouchscreen` + KDE `KWIN_IM_SHOW_ALWAYS=0`.
    /// Manual `hyprosk show` / `toggle` always still works
    #[serde(default)]
    pub touch_only: bool,
    /// Haptic/audio feedback command on keypress (e.g. "paplay /path/to/click.ogg" or "feedbackd")
    pub feedback_command: Option<String>,
    /// Long press timeout in milliseconds for alternate characters
    pub long_press_ms: u64,
    /// Repeat delay in ms
    pub repeat_delay_ms: u64,
    /// Repeat rate in ms
    pub repeat_rate_ms: u64,
    /// Read surrounding text from input-method-v2. Defaults to false so that
    /// buggy browser/Helium host text readback does not interfere with
    /// typing, while preserving input-method commits and virtual-keyboard writes.
    #[serde(default = "default_false")]
    pub surrounding_text_readback: bool,
}

fn default_false() -> bool {
    false
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            auto_show: true,
            auto_hide: true,
            hide_on_fullscreen: true,
            folio_mode: false,
            touch_only: false,
            feedback_command: None,
            long_press_ms: 400,
            repeat_delay_ms: 350,
            repeat_rate_ms: 45,
            surrounding_text_readback: false,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            theme: ThemeConfig::default(),
            behavior: BehaviorConfig::default(),
        }
    }
}

impl Config {
    pub fn default_config_path() -> PathBuf {
        if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg_config).join("hyprosk/config.toml")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config/hyprosk/config.toml")
        } else {
            PathBuf::from("config.toml")
        }
    }

    pub fn load_or_create(path: Option<&Path>) -> Self {
        let config_path = path
            .map(Path::to_path_buf)
            .unwrap_or_else(Self::default_config_path);

        if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(content) => match toml::from_str(&content) {
                    Ok(config) => {
                        tracing::info!("Loaded configuration from {:?}", config_path);
                        return config;
                    }
                    Err(e) => {
                        tracing::error!("Failed to parse config file at {:?}: {}", config_path, e);
                    }
                },
                Err(e) => {
                    tracing::error!("Failed to read config file at {:?}: {}", config_path, e);
                }
            }
        }

        let default_config = Self::default();
        if let Some(parent) = config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(serialized) = toml::to_string_pretty(&default_config) {
            let _ = std::fs::write(&config_path, serialized);
            tracing::info!("Generated default config at {:?}", config_path);
        }

        default_config
    }
}
