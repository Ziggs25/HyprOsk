use crate::layout::{key::Key, KeyboardLayout};
use crate::render::theme::{Color, Theme};

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= (self.x + self.w) && py >= self.y && py <= (self.y + self.h)
    }
}

pub struct RenderEngine;

impl RenderEngine {
    pub fn calculate_key_rects(
        layout: &KeyboardLayout,
        total_width: u32,
        total_height: u32,
        _theme: &Theme,
    ) -> Vec<Vec<(Rect, usize)>> {
        let is_mobile = layout.is_mobile() || total_width < 600;
        let padding_x = layout.padding_x.unwrap_or(if is_mobile { 3.5 } else { 8.0 });
        let padding_y = layout.padding_y.unwrap_or(if is_mobile { 4.0 } else { 8.0 });
        let spacing = layout.key_spacing.unwrap_or(if is_mobile { 4.0 } else { 8.0 });
        let row_spacing = layout.row_spacing.unwrap_or(spacing);
        let num_rows = layout.rows.len();

        if num_rows == 0 {
            return Vec::new();
        }

        // Top action bar height: 38px on portrait/mobile, 46px on landscape.
        let top_row_height = if is_mobile { 38.0 } else { 46.0 };
        let remaining_height = (total_height as f32 - (2.0 * padding_y) - top_row_height - (num_rows as f32 * row_spacing)).max(10.0);
        let default_key_row_height = layout.key_height.unwrap_or((remaining_height / (num_rows - 1).max(1) as f32).max(10.0));

        let mut result = Vec::with_capacity(num_rows);

        let mut current_y = padding_y;

        for (r_idx, row) in layout.rows.iter().enumerate() {
            let row_height = if r_idx == 0 { top_row_height } else { default_key_row_height };
            let mut row_rects = Vec::with_capacity(row.keys.len());

            if r_idx == 0 && row.keys.len() >= 2 {
                let num_center = row.keys.len() - 2;
                let action_width = if is_mobile {
                    // In portrait/mobile, action buttons (clip, hide) are compact
                    (row_height * 0.95).round().min(36.0)
                } else {
                    ((total_width as f32 - 2.0 * padding_x - 4.0 * spacing) * (0.35 / (2.0 * 0.35 + 3.3))).round()
                };

                // Left button: Clipboard
                let x0 = padding_x;
                row_rects.push((Rect { x: x0, y: current_y, w: action_width, h: row_height }, 0));

                // Center suggestion region
                let center_start_x = x0 + action_width + spacing;
                let right_start_x = total_width as f32 - padding_x - action_width;
                let center_avail_w = (right_start_x - center_start_x - spacing).max(10.0);

                if num_center > 0 {
                    let center_total_weight: f32 = row.keys[1..1 + num_center].iter().map(|k| k.width_weight).sum();
                    let center_spacing_total = (num_center.saturating_sub(1)) as f32 * spacing;
                    let center_keys_avail_w = (center_avail_w - center_spacing_total).max(10.0);

                    let mut curr_cx = center_start_x;
                    for (c_idx, key) in row.keys[1..1 + num_center].iter().enumerate() {
                        let w = (center_keys_avail_w * (key.width_weight / center_total_weight)).max(10.0);
                        row_rects.push((Rect { x: curr_cx, y: current_y, w, h: row_height }, 1 + c_idx));
                        curr_cx += w + spacing;
                    }
                }

                // Right button: Hide
                let x_hide = right_start_x;
                row_rects.push((Rect { x: x_hide, y: current_y, w: action_width, h: row_height }, 1 + num_center));
            } else {
                let total_weights: f32 = row.keys.iter().map(|k| k.width_weight).sum();
                let num_keys = row.keys.len();

                // Sum of explicit extra margins on keys
                let explicit_key_margins: f32 = row.keys.iter().map(|k| {
                    k.margin_left.unwrap_or(0.0) + k.margin_right.unwrap_or(0.0)
                }).sum();

                // Check if any key has explicit px width or layout has key_width
                let has_explicit_key_w = layout.key_width.is_some() || row.keys.iter().any(|k| k.width_px.is_some());

                // Gboard / HeliBoard: Row 2 has 9 keys (a-l) centered against Row 1's 10 keys (q-p).
                // When in mobile/portrait and a row has 9 keys of weight 1.0, use the exact single-key width
                // from a 10-key row and center the entire 9-key row with equal left and right margins!
                let is_centered_9_key_row = !has_explicit_key_w
                    && is_mobile
                    && num_keys == 9
                    && (total_weights - 9.0).abs() < 0.05
                    && explicit_key_margins == 0.0;

                if is_centered_9_key_row {
                    let w_10 = total_width as f32 - (2.0 * padding_x) - (9.0 * spacing);
                    let single_key_w = (w_10 / 10.0).max(10.0);
                    let row_w = 9.0 * single_key_w + 8.0 * spacing;
                    let indent = ((total_width as f32 - row_w) * 0.5).max(padding_x);

                    let mut current_x = indent;
                    for (k_idx, key) in row.keys.iter().enumerate() {
                        let final_h = key.height.unwrap_or(row_height);
                        let rect = Rect {
                            x: current_x,
                            y: current_y,
                            w: single_key_w,
                            h: final_h,
                        };
                        row_rects.push((rect, k_idx));
                        current_x += single_key_w + spacing;
                    }
                } else {
                    let base_spacing_total = (num_keys.saturating_sub(1)) as f32 * spacing;
                    let avail_width = total_width as f32 - (2.0 * padding_x) - base_spacing_total - explicit_key_margins;
                    let mut current_x = padding_x;

                    for (k_idx, key) in row.keys.iter().enumerate() {
                        if let Some(ml) = key.margin_left {
                            current_x += ml;
                        }
                        let key_width = if let Some(w_px) = key.width_px {
                            w_px
                        } else if let Some(def_kw) = layout.key_width {
                            (def_kw * key.width_weight).max(10.0)
                        } else {
                            (avail_width * (key.width_weight / total_weights)).max(10.0)
                        };
                        let final_h = key.height.unwrap_or(row_height);
                        let rect = Rect {
                            x: current_x,
                            y: current_y,
                            w: key_width,
                            h: final_h,
                        };
                        row_rects.push((rect, k_idx));
                        current_x += key_width;
                        if let Some(mr) = key.margin_right {
                            current_x += mr;
                        }
                        if k_idx + 1 < num_keys {
                            current_x += spacing;
                        }
                    }
                }
            }

            let v_gap = row.margin_bottom.unwrap_or(row_spacing);
            current_y += row_height + v_gap;
            result.push(row_rects);
        }

        result
    }

    pub fn hit_test<'a>(
        layout: &'a KeyboardLayout,
        key_rects: &[Vec<(Rect, usize)>],
        x: f64,
        y: f64,
    ) -> Option<(usize, usize, &'a Key)> {
        let px = x as f32;
        let py = y as f32;

        for (r_idx, row_rects) in key_rects.iter().enumerate() {
            for (rect, k_idx) in row_rects {
                if rect.contains(px, py)
                    && let Some(row) = layout.rows.get(r_idx)
                    && let Some(key) = row.keys.get(*k_idx)
                {
                    return Some((r_idx, *k_idx, key));
                }
            }
        }
        None
    }

    pub fn render(
        pixels: &mut [u8],
        width: u32,
        height: u32,
        layout: &KeyboardLayout,
        theme: &Theme,
        pressed_keys: &[(usize, usize)],
        latched_keys: &[(usize, usize)],
        swipe_offset: Option<f32>,
    ) {
        let mut bg = theme.background;
        if layout.blur.unwrap_or(false) {
            let opacity = layout.opacity.unwrap_or(0.60);
            bg.a = (opacity * 255.0).clamp(0.0, 255.0) as u8;
        }
        let bg_u32 = bg.to_argb_u32();
        let pixel_u32_slice: &mut [u32] = unsafe {
            std::slice::from_raw_parts_mut(pixels.as_mut_ptr() as *mut u32, (width * height) as usize)
        };
        pixel_u32_slice.fill(bg_u32);

        let key_rects = Self::calculate_key_rects(layout, width, height, theme);

        for (r_idx, row) in layout.rows.iter().enumerate() {
            let row_rects = match key_rects.get(r_idx) {
                Some(r) => r,
                None => continue,
            };

            let is_suggestion_bar = r_idx == 0;

            for (rect, k_idx) in row_rects {
                let key = match row.keys.get(*k_idx) {
                    Some(k) => k,
                    None => continue,
                };

                let is_pressed = pressed_keys.contains(&(r_idx, *k_idx));
                let is_latched = latched_keys.contains(&(r_idx, *k_idx));
                let is_active = is_pressed || is_latched;
                // Determine border width and whether borders are enabled (borderless if borders == false)
                let borders_enabled = key.borders.or(layout.borders).unwrap_or(true);
                let border_w = if !borders_enabled {
                    0.0
                } else if is_suggestion_bar {
                    0.5
                } else {
                    key.border_width.or(layout.border_width).unwrap_or(theme.border_width)
                };

                let mut key_bg = if is_active {
                    theme.key_pressed
                } else if is_suggestion_bar {
                    if key.is_special {
                        // Highlighted best suggestion candidate in center
                        theme.key_special
                    } else {
                        Color::rgba(theme.key_background.r, theme.key_background.g, theme.key_background.b, 120)
                    }
                } else if !borders_enabled && !key.is_special {
                    bg
                } else if key.is_special {
                    theme.key_special
                } else {
                    theme.key_background
                };

                if layout.blur.unwrap_or(false) && !is_active && (!is_suggestion_bar || key.is_special) {
                    let key_alpha = (layout.key_opacity.unwrap_or(0.50) * 255.0).clamp(0.0, 255.0) as u8;
                    key_bg.a = key_alpha;
                }

                let text_col = if is_active {
                    Color::rgb(255, 255, 255)
                } else if is_suggestion_bar && key.is_special {
                    theme.accent_color
                } else if key.is_special {
                    theme.text_special
                } else {
                    theme.text_color
                };

                let radius = if is_suggestion_bar { 6.0 } else { theme.key_radius };

                // Draw key surface
                Self::draw_rounded_rect(
                    pixel_u32_slice,
                    width,
                    height,
                    *rect,
                    radius,
                    key_bg,
                    theme.border_color,
                    border_w,
                );

                // Draw label text
                let base_scale = if !is_suggestion_bar && key.label.len() <= 2 { 2 } else { 1 };
                let mut primary_scale = key.font_size
                    .or(layout.font_size)
                    .map(|f| f.round().max(1.0) as i32)
                    .unwrap_or(base_scale);

                // Auto-clamp font scale if the text width would overflow the key's width
                if !is_suggestion_bar {
                    let char_count = key.label.chars().count() as f32;
                    while primary_scale > 1 {
                        let text_w = char_count * (8.0 * primary_scale as f32 + primary_scale as f32);
                        if text_w <= rect.w - 4.0 {
                            break;
                        }
                        primary_scale -= 1;
                    }
                }

                Self::draw_simple_text(
                    pixel_u32_slice,
                    width,
                    height,
                    &key.label,
                    rect.x + (rect.w / 2.0),
                    rect.y + (rect.h / 2.0),
                    text_col,
                    primary_scale,
                );

                // Draw secondary label (top-right symbol/number hint)
                if let Some(ref sec) = key.secondary_label {
                    let sec_scale = key.secondary_font_size
                        .or(layout.secondary_font_size)
                        .map(|f| f.round().max(1.0) as i32)
                        .unwrap_or(1);

                    Self::draw_simple_text(
                        pixel_u32_slice,
                        width,
                        height,
                        sec,
                        rect.x + rect.w - (9.0 + (sec_scale - 1) as f32 * 4.0),
                        rect.y + (9.0 + (sec_scale - 1) as f32 * 3.0),
                        Color::rgba(text_col.r, text_col.g, text_col.b, 130),
                        sec_scale,
                    );
                }

                // If this is the spacebar and the user is swiping to move cursor, draw glide indicator
                if let Some(offset) = swipe_offset
                    && (key.label.contains("English") || key.label.contains("␣"))
                {
                    let indicator_x = (rect.x + (rect.w / 2.0) + offset).clamp(rect.x + 10.0, rect.x + rect.w - 10.0);
                    Self::draw_glide_indicator(pixel_u32_slice, width, height, indicator_x, rect.y + rect.h - 6.0, theme.accent_color);
                }
            }
        }
    }

    fn draw_glide_indicator(pixels: &mut [u32], stride: u32, max_h: u32, cx: f32, cy: f32, color: Color) {
        let col_u32 = color.to_argb_u32();
        let ix = cx as i32;
        let iy = cy as i32;
        for dy in -2..=2 {
            for dx in -14..=14 {
                let px = ix + dx;
                let py = iy + dy;
                if px >= 0 && px < stride as i32 && py >= 0 && py < max_h as i32 {
                    let idx = (py as u32 * stride + px as u32) as usize;
                    if idx < pixels.len() {
                        pixels[idx] = col_u32;
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_rounded_rect(
        pixels: &mut [u32],
        stride: u32,
        max_h: u32,
        rect: Rect,
        radius: f32,
        color: Color,
        border_color: Color,
        border_w: f32,
    ) {
        let x0 = rect.x.clamp(0.0, stride as f32) as u32;
        let y0 = rect.y.clamp(0.0, max_h as f32) as u32;
        let x1 = (rect.x + rect.w).clamp(0.0, stride as f32) as u32;
        let y1 = (rect.y + rect.h).clamp(0.0, max_h as f32) as u32;

        let col_u32 = color.to_argb_u32();
        let border_u32 = border_color.to_argb_u32();
        let r2 = radius * radius;

        for y in y0..y1 {
            for x in x0..x1 {
                let fx = x as f32;
                let fy = y as f32;

                let in_corner_tl = fx < rect.x + radius && fy < rect.y + radius;
                let in_corner_tr = fx > rect.x + rect.w - radius && fy < rect.y + radius;
                let in_corner_bl = fx < rect.x + radius && fy > rect.y + rect.h - radius;
                let in_corner_br = fx > rect.x + rect.w - radius && fy > rect.y + rect.h - radius;

                let mut is_inside = true;

                if in_corner_tl {
                    let dx = fx - (rect.x + radius);
                    let dy = fy - (rect.y + radius);
                    if (dx * dx + dy * dy) > r2 { is_inside = false; }
                } else if in_corner_tr {
                    let dx = fx - (rect.x + rect.w - radius);
                    let dy = fy - (rect.y + radius);
                    if (dx * dx + dy * dy) > r2 { is_inside = false; }
                } else if in_corner_bl {
                    let dx = fx - (rect.x + radius);
                    let dy = fy - (rect.y + rect.h - radius);
                    if (dx * dx + dy * dy) > r2 { is_inside = false; }
                } else if in_corner_br {
                    let dx = fx - (rect.x + rect.w - radius);
                    let dy = fy - (rect.y + rect.h - radius);
                    if (dx * dx + dy * dy) > r2 { is_inside = false; }
                }

                if is_inside {
                    let is_border = border_w > 0.0 && (
                        fx < rect.x + border_w || fx > rect.x + rect.w - border_w ||
                        fy < rect.y + border_w || fy > rect.y + rect.h - border_w
                    );
                    let idx = (y * stride + x) as usize;
                    if idx < pixels.len() {
                        pixels[idx] = if is_border { border_u32 } else { col_u32 };
                    }
                }
            }
        }
    }

    fn draw_simple_text(
        pixels: &mut [u32],
        stride: u32,
        max_h: u32,
        text: &str,
        cx: f32,
        cy: f32,
        color: Color,
        scale: i32,
    ) {
        let scale = scale.max(1);
        let char_w = 8 * scale;
        let char_h = 8 * scale;
        let total_w = (text.chars().count() as i32) * (char_w + scale);
        let start_x = (cx - (total_w as f32 / 2.0)) as i32;
        let start_y = (cy - (char_h as f32 / 2.0)) as i32;
        let color_u32 = color.to_argb_u32();

        for (i, ch) in text.chars().enumerate() {
            let ox = start_x + (i as i32 * (char_w + scale));
            let bitmap = get_glyph_bitmap(ch);

            for (row, &row_bits) in bitmap.iter().enumerate() {
                for col in 0..8i32 {
                    if (row_bits & (1 << (7 - col))) != 0 {
                        for sy in 0..scale {
                            for sx in 0..scale {
                                let px = ox + (col * scale) + sx;
                                let py = start_y + (row as i32 * scale) + sy;
                                if px >= 0 && px < stride as i32 && py >= 0 && py < max_h as i32 {
                                    let idx = (py as u32 * stride + px as u32) as usize;
                                    if idx < pixels.len() {
                                        pixels[idx] = color_u32;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn get_glyph_bitmap(ch: char) -> [u8; 8] {
    match ch {
        'a' | 'A' => [0x18, 0x24, 0x42, 0x7E, 0x42, 0x42, 0x42, 0x00],
        'b' | 'B' => [0x7C, 0x42, 0x42, 0x7C, 0x42, 0x42, 0x7C, 0x00],
        'c' | 'C' => [0x3C, 0x42, 0x40, 0x40, 0x40, 0x42, 0x3C, 0x00],
        'd' | 'D' => [0x78, 0x44, 0x42, 0x42, 0x42, 0x44, 0x78, 0x00],
        'e' | 'E' => [0x7E, 0x40, 0x40, 0x7C, 0x40, 0x40, 0x7E, 0x00],
        'f' | 'F' => [0x7E, 0x40, 0x40, 0x7C, 0x40, 0x40, 0x40, 0x00],
        'g' | 'G' => [0x3C, 0x42, 0x40, 0x4E, 0x42, 0x42, 0x3C, 0x00],
        'h' | 'H' => [0x42, 0x42, 0x42, 0x7E, 0x42, 0x42, 0x42, 0x00],
        'i' | 'I' => [0x38, 0x10, 0x10, 0x10, 0x10, 0x10, 0x38, 0x00],
        'j' | 'J' => [0x1C, 0x08, 0x08, 0x08, 0x08, 0x48, 0x30, 0x00],
        'k' | 'K' => [0x44, 0x48, 0x50, 0x60, 0x50, 0x48, 0x44, 0x00],
        'l' | 'L' => [0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x7E, 0x00],
        'm' | 'M' => [0x42, 0x66, 0x5A, 0x42, 0x42, 0x42, 0x42, 0x00],
        'n' | 'N' => [0x42, 0x62, 0x52, 0x4A, 0x46, 0x42, 0x42, 0x00],
        'o' | 'O' => [0x3C, 0x42, 0x42, 0x42, 0x42, 0x42, 0x3C, 0x00],
        'p' | 'P' => [0x7C, 0x42, 0x42, 0x7C, 0x40, 0x40, 0x40, 0x00],
        'q' | 'Q' => [0x3C, 0x42, 0x42, 0x42, 0x4A, 0x44, 0x3A, 0x00],
        'r' | 'R' => [0x7C, 0x42, 0x42, 0x7C, 0x50, 0x48, 0x44, 0x00],
        's' | 'S' => [0x3C, 0x42, 0x40, 0x3C, 0x02, 0x42, 0x3C, 0x00],
        't' | 'T' => [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00],
        'u' | 'U' => [0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x3C, 0x00],
        'v' | 'V' => [0x42, 0x42, 0x42, 0x42, 0x42, 0x24, 0x18, 0x00],
        'w' | 'W' => [0x42, 0x42, 0x42, 0x5A, 0x5A, 0x66, 0x42, 0x00],
        'x' | 'X' => [0x42, 0x24, 0x18, 0x18, 0x18, 0x24, 0x42, 0x00],
        'y' | 'Y' => [0x42, 0x42, 0x24, 0x18, 0x18, 0x18, 0x18, 0x00],
        'z' | 'Z' => [0x7E, 0x04, 0x08, 0x10, 0x20, 0x40, 0x7E, 0x00],
        '0' => [0x3C, 0x46, 0x4A, 0x52, 0x62, 0x42, 0x3C, 0x00],
        '1' => [0x18, 0x28, 0x08, 0x08, 0x08, 0x08, 0x3E, 0x00],
        '2' => [0x3C, 0x42, 0x02, 0x0C, 0x30, 0x40, 0x7E, 0x00],
        '3' => [0x3C, 0x42, 0x02, 0x1C, 0x02, 0x42, 0x3C, 0x00],
        '4' => [0x08, 0x18, 0x28, 0x48, 0x7E, 0x08, 0x08, 0x00],
        '5' => [0x7E, 0x40, 0x7C, 0x02, 0x02, 0x42, 0x3C, 0x00],
        '6' => [0x3C, 0x40, 0x7C, 0x42, 0x42, 0x42, 0x3C, 0x00],
        '7' => [0x7E, 0x02, 0x04, 0x08, 0x10, 0x20, 0x20, 0x00],
        '8' => [0x3C, 0x42, 0x42, 0x3C, 0x42, 0x42, 0x3C, 0x00],
        '⇧' => [0x18, 0x3C, 0x7E, 0xFF, 0x18, 0x18, 0x18, 0x00],
        '⇪' => [0x18, 0x3C, 0x7E, 0xFF, 0x18, 0x18, 0x00, 0x7E],
        '⌫' => [0x0C, 0x1A, 0x36, 0x66, 0x66, 0x36, 0x1A, 0x0C],
        '⏎' => [0x02, 0x02, 0x02, 0x3E, 0x42, 0x1C, 0x08, 0x00],
        '␣' => [0x00, 0x00, 0x00, 0x00, 0x42, 0x42, 0x7E, 0x00],
        '▼' => [0x00, 0x00, 0xFF, 0x7E, 0x3C, 0x18, 0x00, 0x00],
        '▲' => [0x00, 0x18, 0x3C, 0x7E, 0xFF, 0x00, 0x00, 0x00],
        '◀' => [0x08, 0x18, 0x38, 0x78, 0x38, 0x18, 0x08, 0x00],
        '▶' => [0x10, 0x18, 0x1C, 0x1E, 0x1C, 0x18, 0x10, 0x00],
        '‹' => [0x04, 0x08, 0x10, 0x20, 0x10, 0x08, 0x04, 0x00],
        '›' => [0x20, 0x10, 0x08, 0x04, 0x08, 0x10, 0x20, 0x00],
        '?' => [0x3C, 0x42, 0x02, 0x0C, 0x10, 0x00, 0x10, 0x00],
        '!' => [0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x18, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x10],
        ':' => [0x00, 0x18, 0x18, 0x00, 0x00, 0x18, 0x18, 0x00],
        ';' => [0x00, 0x18, 0x18, 0x00, 0x00, 0x18, 0x18, 0x10],
        '/' => [0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x00],
        '=' => [0x00, 0x7E, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00],
        '+' => [0x00, 0x18, 0x18, 0x7E, 0x18, 0x18, 0x00, 0x00],
        '-' => [0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00],
        '@' => [0x3C, 0x42, 0x9A, 0xA6, 0xA2, 0x40, 0x3C, 0x00],
        '#' => [0x24, 0x24, 0x7E, 0x24, 0x7E, 0x24, 0x24, 0x00],
        '$' => [0x10, 0x3C, 0x50, 0x38, 0x14, 0x78, 0x10, 0x00],
        '%' => [0x62, 0x64, 0x08, 0x10, 0x20, 0x4C, 0x8C, 0x00],
        '&' => [0x38, 0x44, 0x38, 0x54, 0x92, 0x92, 0x6C, 0x00],
        '*' => [0x00, 0x24, 0x18, 0x7E, 0x18, 0x24, 0x00, 0x00],
        '(' => [0x08, 0x10, 0x20, 0x20, 0x20, 0x10, 0x08, 0x00],
        ')' => [0x20, 0x10, 0x08, 0x08, 0x08, 0x10, 0x20, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x00],
        _ => [0x7E, 0x42, 0x42, 0x42, 0x42, 0x42, 0x7E, 0x00],
    }
}
