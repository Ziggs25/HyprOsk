use crate::config::ThemeConfig;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn parse_hex(hex: &str) -> Self {
        let hex = hex.trim_start_matches('#');
        match hex.len() {
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
                Self::rgb(r, g, b)
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
                let a = u8::from_str_radix(&hex[6..8], 16).unwrap_or(255);
                Self::rgba(r, g, b, a)
            }
            _ => Self::rgb(255, 255, 255),
        }
    }

    #[inline]
    pub fn to_argb_u32(self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    #[inline]
    pub fn blend_over(self, background: Color) -> Color {
        let alpha = self.a as f32 / 255.0;
        let inv_alpha = 1.0 - alpha;
        Color {
            r: (self.r as f32 * alpha + background.r as f32 * inv_alpha) as u8,
            g: (self.g as f32 * alpha + background.g as f32 * inv_alpha) as u8,
            b: (self.b as f32 * alpha + background.b as f32 * inv_alpha) as u8,
            a: 255,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Color,
    pub key_background: Color,
    pub key_pressed: Color,
    pub key_special: Color,
    pub text_color: Color,
    pub text_special: Color,
    pub accent_color: Color,
    pub border_color: Color,
    pub border_width: f32,
    pub key_radius: f32,
    pub key_spacing: f32,
}

impl From<&ThemeConfig> for Theme {
    fn from(cfg: &ThemeConfig) -> Self {
        Self {
            background: Color::parse_hex(&cfg.background),
            key_background: Color::parse_hex(&cfg.key_background),
            key_pressed: Color::parse_hex(&cfg.key_pressed),
            key_special: Color::parse_hex(&cfg.key_special),
            text_color: Color::parse_hex(&cfg.text_color),
            text_special: Color::parse_hex(&cfg.text_special),
            accent_color: Color::parse_hex(&cfg.accent_color),
            border_color: Color::parse_hex(&cfg.border_color),
            border_width: cfg.border_width,
            key_radius: cfg.key_radius,
            key_spacing: cfg.key_spacing,
        }
    }
}

impl Theme {
    pub fn catppuccin_mocha() -> Self {
        Self {
            background: Color::rgba(30, 30, 46, 230),
            key_background: Color::rgba(49, 50, 68, 255),
            key_pressed: Color::rgba(137, 180, 250, 255),
            key_special: Color::rgba(69, 71, 90, 255),
            text_color: Color::rgb(205, 214, 244),
            text_special: Color::rgb(245, 224, 220),
            accent_color: Color::rgb(203, 166, 247),
            border_color: Color::rgba(88, 91, 112, 100),
            border_width: 1.0,
            key_radius: 8.0,
            key_spacing: 6.0,
        }
    }

    pub fn tokyo_night() -> Self {
        Self {
            background: Color::rgba(26, 27, 38, 235),
            key_background: Color::rgba(36, 40, 59, 255),
            key_pressed: Color::rgba(122, 162, 247, 255),
            key_special: Color::rgba(41, 46, 66, 255),
            text_color: Color::rgb(192, 202, 245),
            text_special: Color::rgb(224, 175, 104),
            accent_color: Color::rgb(187, 154, 247),
            border_color: Color::rgba(65, 72, 104, 120),
            border_width: 1.0,
            key_radius: 8.0,
            key_spacing: 6.0,
        }
    }

    pub fn oled_dark() -> Self {
        Self {
            background: Color::rgba(0, 0, 0, 245),
            key_background: Color::rgba(24, 24, 24, 255),
            key_pressed: Color::rgba(0, 150, 255, 255),
            key_special: Color::rgba(38, 38, 38, 255),
            text_color: Color::rgb(240, 240, 240),
            text_special: Color::rgb(255, 215, 0),
            accent_color: Color::rgb(0, 200, 255),
            border_color: Color::rgba(60, 60, 60, 120),
            border_width: 1.0,
            key_radius: 8.0,
            key_spacing: 6.0,
        }
    }
}
