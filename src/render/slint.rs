//! Slint headless paint bridge.
//!
//! Slint is used strictly as a paint bucket: it renders the Wireframe
//! (`keyboard wireframe.md`) scene into the existing Wayland `wl_shm` ARGB
//! buffer. All layout geometry, hit-testing, gesture state machines and input
//! output stay in Rust (`RenderEngine::calculate_key_rects` is the source of
//! geometry truth).

use std::cell::Cell;
use std::rc::Rc;

use slint::platform::{
    software_renderer::{PremultipliedRgbaColor, SoftwareRenderer},
    Platform, PlatformError, Renderer, WindowAdapter, WindowEvent,
};
use slint::{LogicalSize, ModelRc, PhysicalSize, VecModel, Window};

use crate::layout::KeyboardLayout;
use crate::layout::key::KeyAction;
use crate::render::engine::RenderEngine;

slint::include_modules!();

/// Icon codes shared with `ui/osk.slint` `Key.icon`.
pub mod icon {
    pub const BACKSPACE: i32 = 1;
    pub const ENTER: i32 = 2;
    pub const SHIFT: i32 = 3;
    pub const WIN: i32 = 4;
    pub const MIC: i32 = 5;
    pub const ARROW_L: i32 = 6;
    pub const ARROW_R: i32 = 7;
    pub const ARROW_U: i32 = 8;
    pub const ARROW_D: i32 = 9;
}

const BACKSPACE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="{COLOR}" d="M22 3H7c-.69 0-1.23.35-1.59.88L0 12l5.41 8.11c.36.53.9.89 1.59.89h15c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm0 16H7.07L2.4 12l4.66-7H22v14zm-11.59-2L14 13.41 17.59 17 19 15.59 15.41 12 19 8.41 17.59 7 14 10.59 10.41 7 9 8.41 12.59 12 9 15.59z"/></svg>"#;
const ENTER_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="{COLOR}" d="M19 7v4H5.83l3.58-3.59L8 6l-6 6 6 6 1.41-1.41L5.83 13H21V7h-2z"/></svg>"#;
const SHIFT_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="none" stroke="{COLOR}" stroke-width="2" d="M4 15h6v5h4v-5h6L12 4 4 15z"/></svg>"#;
const WIN_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="{COLOR}" d="M3 3h8v8H3V3zm10 0h8v8h-8V3zM3 13h8v8H3v-8zm10 0h8v8h-8v-8z"/></svg>"#;
const MIC_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="{COLOR}" d="M12 14c1.66 0 3-1.34 3-3V5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3zm5-3c0 2.76-2.24 5-5 5s-5-2.24-5-5H5c0 3.53 2.61 6.43 6 6.92V21h2v-2.08c3.39-.49 6-3.39 6-6.92h-2z"/></svg>"#;
const ARROW_L_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="{COLOR}" d="M15.41 7.41 14 6l-6 6 6 6 1.41-1.41L10.83 12z"/></svg>"#;
const ARROW_R_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="{COLOR}" d="M8.59 16.59 10 18l6-6-6-6-1.41 1.41L13.17 12z"/></svg>"#;
const ARROW_U_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="{COLOR}" d="M4 15h6v5h4v-5h6L12 4 4 15z"/></svg>"#;
const ARROW_D_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="{COLOR}" d="M20 9l-8 8-8-8z"/></svg>"#;
const GEAR_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="{COLOR}" d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.58-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"/></svg>"#;
const PALETTE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="{COLOR}" d="M12 3c-4.97 0-9 4.03-9 9 0 2.12.74 4.07 1.97 5.61.35.43.91.66 1.46.54.51-.11.89-.52.94-1.04.08-.85.76-1.51 1.63-1.51h1.5c3.03 0 5.5-2.47 5.5-5.5 0-3.92-3.13-7.1-7-7.1zm-4.5 9c-.83 0-1.5-.67-1.5-1.5S6.67 9 7.5 9 9 9.67 9 10.5 8.33 12 7.5 12zm3-4C9.67 8 9 7.33 9 6.5S9.67 5 10.5 5s1.5.67 1.5 1.5S11.33 8 10.5 8zm3 0c-.83 0-1.5-.67-1.5-1.5S12.67 5 13.5 5s1.5.67 1.5 1.5S14.33 8 13.5 8zm3 4c-.83 0-1.5-.67-1.5-1.5S15.67 9 16.5 9s1.5.67 1.5 1.5-.67 1.5-1.5 1.5z"/></svg>"#;
const DOCK_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="{COLOR}" d="M19 3H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm0 16H5V5h14v14zM7 10h4v7H7v-7zm6 3h4v4h-4v-4zm0-3h4v2h-4v-2z"/></svg>"#;
const DISMISS_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="{COLOR}" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>"#;

fn svg(template: &str, color: &str) -> slint::Image {
    slint::Image::load_from_svg_data(template.replace("{COLOR}", color).as_bytes()).unwrap()
}

struct Adapter {
    window: Rc<Window>,
    renderer: SoftwareRenderer,
    size: Cell<PhysicalSize>,
}

impl WindowAdapter for Adapter {
    fn window(&self) -> &Window {
        &self.window
    }
    fn size(&self) -> PhysicalSize {
        self.size.get()
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
}

fn make_adapter(size: PhysicalSize) -> Rc<Adapter> {
    Rc::<Adapter>::new_cyclic(|weak| Adapter {
        window: Rc::new(Window::new(weak.clone())),
        renderer: SoftwareRenderer::new(),
        size: Cell::new(size),
    })
}

struct Sp {
    adapter: Rc<Adapter>,
}

impl Platform for Sp {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.adapter.clone())
    }
}

/// Owns the single Slint platform/adapter/component for the whole daemon.
pub struct SlintScene {
    adapter: Rc<Adapter>,
    ui: OskUi,
    canvas: Vec<PremultipliedRgbaColor>,
    width: u32,
    height: u32,
}

// Wireframe palette is fixed in `ui/osk.slint`; legacy themes are ignored.

impl SlintScene {
    pub fn new(width: u32, height: u32) -> Result<Self, PlatformError> {
        let size = PhysicalSize::new(width, height);
        let adapter = make_adapter(size);
        slint::platform::set_platform(Box::new(Sp { adapter: adapter.clone() }))
            .map_err(|e| PlatformError::from(e.to_string()))?;

        let ui = OskUi::new()?;
        ui.set_icon_backspace(svg(BACKSPACE_SVG, "#fff"));
        ui.set_icon_enter(svg(ENTER_SVG, "#fff"));
        ui.set_icon_shift(svg(SHIFT_SVG, "#fff"));
        ui.set_icon_win(svg(WIN_SVG, "#fff"));
        ui.set_icon_mic(svg(MIC_SVG, "#fff"));
        ui.set_icon_arrow_l(svg(ARROW_L_SVG, "#fff"));
        ui.set_icon_arrow_r(svg(ARROW_R_SVG, "#fff"));
        ui.set_icon_arrow_u(svg(ARROW_U_SVG, "#fff"));
        ui.set_icon_arrow_d(svg(ARROW_D_SVG, "#fff"));
        ui.set_icon_gear(svg(GEAR_SVG, "#fff"));
        ui.set_icon_palette(svg(PALETTE_SVG, "#fff"));
        ui.set_icon_dock(svg(DOCK_SVG, "#fff"));
        ui.set_icon_dismiss(svg(DISMISS_SVG, "#fff"));

        ui.show()?;
        ui.window().dispatch_event(WindowEvent::Resized {
            size: LogicalSize::new(width as f32, height as f32),
        });

        Ok(Self {
            adapter,
            ui,
            canvas: Vec::with_capacity((width * height) as usize),
            width,
            height,
        })
    }

    /// Map a key action (plus label) to the icon code used by `osk.slint`.
    fn icon_for(action: &KeyAction, label: &str) -> i32 {
        match action {
            KeyAction::Backspace => icon::BACKSPACE,
            KeyAction::Enter => icon::ENTER,
            KeyAction::Shift => icon::SHIFT,
            KeyAction::Win => icon::WIN,
            KeyAction::ArrowLeft => icon::ARROW_L,
            KeyAction::ArrowRight => icon::ARROW_R,
            KeyAction::ArrowUp => icon::ARROW_U,
            KeyAction::ArrowDown => icon::ARROW_D,
            KeyAction::None if label == "🎤" => icon::MIC,
            _ => 0,
        }
    }

    /// Renders `layout` into `out_shm` (an ARGB8888 `wl_shm` canvas).
    ///
    /// Returns `true` on success, `false` if the Slint rendering step failed
    /// so the caller can fall back to the legacy `RenderEngine`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        layout: &KeyboardLayout,
        theme: &crate::render::theme::Theme,
        width: u32,
        height: u32,
        pressed_key: Option<(usize, usize)>,
        _swipe_offset: Option<f32>,
        out_shm: &mut [u8],
    ) -> bool {
        // Re-create the rect geometry from the Rust layout (single source of truth).
        let rects = RenderEngine::calculate_key_rects(layout, width, height, theme);

        // Flatten rows and build the Slint key model.
        let mut keys = Vec::new();
        let mut pressed_index: i32 = -1;
        let mut flat = 0i32;
        for (r_idx, row) in rects.iter().enumerate() {
            for (k_idx, (rect, _)) in row.iter().enumerate() {
                let Some(key) = layout.rows.get(r_idx).and_then(|r| r.keys.get(k_idx)) else {
                    continue;
                };
                let icon = Self::icon_for(&key.action, &key.label);
                if pressed_key == Some((r_idx, k_idx)) {
                    pressed_index = flat;
                }
                keys.push(Key {
                    x: rect.x,
                    y: rect.y,
                    w: rect.w,
                    h: rect.h,
                    label: (if icon > 0 { "" } else { key.label.as_str() }).into(),
                    sub: key.secondary_label.clone().unwrap_or_default().into(),
                    icon,
                    is_pressed: pressed_index == flat,
                    is_suggestion: key.is_suggestion,
                });
                flat += 1;
            }
        }

        // Let animations/timers tick before rendering the frame.
        slint::platform::update_timers_and_animations();

        self.ui.set_keys(ModelRc::new(VecModel::from(keys)));
        self.ui.set_pressed_index(pressed_index);
        if width != self.width || height != self.height {
            self.width = width;
            self.height = height;
        }
        self.adapter.size.set(PhysicalSize::new(width, height));
        self.ui.window().dispatch_event(WindowEvent::Resized {
            size: LogicalSize::new(width as f32, height as f32),
        });

        // Wireframe palette is fixed in `ui/osk.slint`; legacy themes are ignored.
        self.canvas
            .resize((width * height) as usize, PremultipliedRgbaColor::default());
        self.adapter.renderer.render(&mut self.canvas, width as usize);

        for (dst, src_px) in premul_chunks(out_shm, width, height).iter_mut().zip(self.canvas.iter()) {
            // ARGB8888 (native endian): memory bytes are B,G,R,A on LE, and the
            // Slint software renderer yields premultiplied R,G,B,A.
            *dst = (src_px.blue as u32)
                | ((src_px.green as u32) << 8)
                | ((src_px.red as u32) << 16)
                | ((src_px.alpha as u32) << 24);
        }
        true
    }
}

/// Reinterpret the ARGB u8 canvas as a mutable retrieved pixel iterator (`u32`
/// per pixel) without copying.
fn premul_chunks(out: &mut [u8], width: u32, height: u32) -> &mut [u32] {
    let expected = (width as usize) * (height as usize) * 4;
    debug_assert_eq!(out.len(), expected);
    // SAFETY: the canvas is an ARGB u8 buffer whose length is a multiple of 4
    // with 4-byte alignment, matching a u32 slice.
    unsafe {
        std::slice::from_raw_parts_mut(out.as_mut_ptr() as *mut u32, (width * height) as usize)
    }
}