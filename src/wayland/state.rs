use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    seat::{
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        touch::TouchHandler,
        Capability, SeatHandler, SeatState,
    },
    shell::wlr_layer::{
        LayerShell, LayerSurface,
    },
    shell::WaylandSurface,
    shm::{
        slot::{Buffer, SlotPool},
        Shm, ShmHandler,
    },
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, delegate_touch, registry_handlers,
};
use std::collections::VecDeque;
use std::os::fd::AsFd;
use std::process::Command;
use std::time::Instant;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
use wayland_client::{
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface, wl_touch, wl_shm},
    Connection, QueueHandle,
};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::ZwpInputMethodV2,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

use crate::config::Config;
use crate::layout::{key::KeyAction, KeyboardLayout, LayerId};
use crate::render::engine::RenderEngine;
use crate::render::slint::SlintScene;
use crate::render::theme::Theme;
use crate::suggest::SuggestEngine;
use crate::wayland::input_method::InputMethodState;
use crate::wayland::layer_shell::create_layer_surface;
use crate::wayland::virtual_keyboard::{create_keymap_fd, KEYMAP_FORMAT};

// Common Linux evdev keycodes
const KEY_ESC: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_TAB: u32 = 15;
const KEY_ENTER: u32 = 28;
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTALT: u32 = 56;
const KEY_SPACE: u32 = 57;
const KEY_LEFTMETA: u32 = 125; // Win / Super
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_UP: u32 = 103;
const KEY_DOWN: u32 = 108;
const KEY_HOME: u32 = 102;
const KEY_END: u32 = 107;

/// Number of SHM buffers to cycle through. The compositor holds a buffer until
/// it releases it; a small ring decouples rendering from the release round-trip
/// while capping pool growth (sctk grows the pool only when the freelist is
/// exhausted).
const IN_FLIGHT_BUFFERS: usize = 3;

pub struct WaylandState {
    // SCTK base states
    pub registry_state: RegistryState,
    pub compositor_state: CompositorState,
    pub output_state: OutputState,
    pub shm_state: Shm,
    pub seat_state: SeatState,
    pub layer_shell: LayerShell,
    pub pool: SlotPool,

    // Wayland Protocol Globals
    pub im_manager: Option<ZwpInputMethodManagerV2>,
    pub input_method: Option<ZwpInputMethodV2>,
    pub vk_manager: Option<ZwpVirtualKeyboardManagerV1>,
    pub virtual_keyboard: Option<ZwpVirtualKeyboardV1>,

    // Surfaces & Lifecycle
    pub layer_surface: Option<LayerSurface>,
    pub is_configured: bool,
    pub is_visible: bool,
    pub manually_shown: bool,
    pub is_running: bool,
    pub width: u32,
    pub height: u32,
    /// True while a physical (folio) keyboard is attached. Gates auto-show in
    /// folio mode; manual show/toggle is unaffected.
    pub folio_attached: bool,

    // Reusable SHM buffers (ring of IN_FLIGHT_BUFFERS slots). Buffers are
    // reused across frames once the compositor releases them; the pool never
    // grows, unlike allocating a fresh buffer per redraw.
    pub canvas_buffers: Vec<Buffer>,
    pub canvas_size: (u32, u32),

    // Input, Suggestions & Gesture State
    pub im_state: InputMethodState,
    pub current_layer: LayerId,
    pub layout: KeyboardLayout,
    pub theme: Theme,
    pub config: Config,
    pub suggest_engine: SuggestEngine,
    pub pressed_key: Option<(usize, usize)>,
    /// Slint headless paint scene (lazily initialized on first redraw).
    pub slint_scene: Option<SlintScene>,

    // Spacebar Glide / Swipe Cursor Navigation
    pub space_touch_start: Option<(f64, f64)>,
    pub space_last_x: f64,
    pub is_space_swiping: bool,
    pub swipe_offset: Option<f32>,

    // Clipboard history view: when active, the key rows are replaced by the
    // clipboard grid and `layout` is built from `clipboard_history`.
    pub clipboard_mode: bool,
    pub clipboard_history: VecDeque<String>,

    // Hold-to-type secondary (sub) characters: press time for the current key.
    pub press_instant: Option<Instant>,
    pub hold_preview: Option<HoldPreview>,
}

#[derive(Debug, Clone)]
pub struct HoldPreview {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub label: String,
}

impl WaylandState {
    pub fn new(
        registry_state: RegistryState,
        compositor_state: CompositorState,
        output_state: OutputState,
        shm_state: Shm,
        seat_state: SeatState,
        layer_shell: LayerShell,
        shm: &Shm,
        config: Config,
    ) -> Self {
        let pool = SlotPool::new(
            (1000 * config.general.height.max(1) * 4) as usize,
            shm,
        ).expect("Failed to create SHM slot pool");
        let theme = Theme::from(&config.theme);
        let height = config.general.height;
        let suggest_engine = SuggestEngine::new();
        let layout = KeyboardLayout::get_layout(LayerId::Lower, &[]);

        Self {
            registry_state,
            compositor_state,
            output_state,
            shm_state,
            seat_state,
            layer_shell,
            pool,
            im_manager: None,
            input_method: None,
            vk_manager: None,
            virtual_keyboard: None,
            layer_surface: None,
            is_configured: false,
            is_visible: false,
            manually_shown: false,
            is_running: true,
            width: 1000,
            height,
            folio_attached: crate::folio::physical_keyboard_attached(),
            canvas_buffers: Vec::new(),
            canvas_size: (0, 0),
            im_state: InputMethodState::default(),
            current_layer: LayerId::Lower,
            layout,
            theme,
            config,
            suggest_engine,
            pressed_key: None,
            slint_scene: None,
            space_touch_start: None,
            space_last_x: 0.0,
            is_space_swiping: false,
            swipe_offset: None,
            clipboard_mode: false,
            clipboard_history: VecDeque::new(),
            press_instant: None,
            hold_preview: None,
        }
    }

    pub fn init_surface(&mut self, qh: &QueueHandle<Self>) {
        if self.layer_surface.is_none() {
            let surface = create_layer_surface(self, qh);
            self.layer_surface = Some(surface);
        }
    }

    pub fn show_keyboard(&mut self, qh: &QueueHandle<Self>) {
        if self.is_visible {
            return;
        }
        tracing::info!("Showing HyprOsk on-screen keyboard");
        self.is_visible = true;
        self.manually_shown = true;
        self.init_surface(qh);

        if let Some(ref surface) = self.layer_surface {
            if self.config.general.exclusive_zone {
                surface.set_exclusive_zone(self.height as i32);
            } else {
                surface.set_exclusive_zone(0);
            }
            surface.commit();
        }

        if self.is_configured {
            self.sync_layout_and_redraw(qh);
        }
    }

    pub fn hide_keyboard(&mut self, _qh: &QueueHandle<Self>) {
        if !self.is_visible {
            return;
        }
        tracing::info!("Hiding HyprOsk on-screen keyboard");
        self.is_visible = false;
        self.is_configured = false;
        self.manually_shown = false;
        if let Some(ref surface) = self.layer_surface {
            surface.set_exclusive_zone(0);
            surface.wl_surface().attach(None, 0, 0);
            surface.commit();
        }
        self.clipboard_mode = false;
        self.suggest_engine.clear();
    }

    pub fn toggle_keyboard(&mut self, qh: &QueueHandle<Self>) {
        if self.is_visible {
            self.hide_keyboard(qh);
        } else {
            self.show_keyboard(qh);
        }
    }

    /// Whether automatic activation (text-field focus) may show the keyboard.
    ///
    /// In folio mode this is denied while a physical keyboard is attached;
    /// manual shows via `hyprosk show` / `toggle` are not affected.
    pub fn allow_auto_show(&self) -> bool {
        if self.config.behavior.folio_mode && self.folio_attached {
            tracing::debug!("Folio keyboard attached, suppressing auto-show");
            false
        } else {
            true
        }
    }

    /// React to a folio attach/detach poll result.
    ///
    /// Detaching has no immediate effect (auto-show resumes on next focus);
    /// attaching hides a keyboard that was only up because no keyboard was
    /// present, leaving manually-shown keyboards alone so debug sessions and
    /// the Folio test runs survive.
    pub fn on_folio_change(&mut self, attached: bool, qh: &QueueHandle<Self>) {
        self.folio_attached = attached;
        tracing::info!("Folio keyboard {} -> auto-show {}", if attached { "attached" } else { "detached" }, if self.allow_auto_show() { "enabled" } else { "suppressed" });
        if self.config.behavior.folio_mode && attached && self.is_visible && !self.manually_shown {
            self.hide_keyboard(qh);
        }
    }

    /// Toggle the clipboard history view. Opening it refreshes the history
    /// with whatever `wl-paste` currently holds.
    pub fn toggle_clipboard(&mut self, qh: &QueueHandle<Self>) {
        self.clipboard_mode = !self.clipboard_mode;
        if self.clipboard_mode {
            self.capture_clipboard();
        }
        self.sync_layout_and_redraw(qh);
    }

    /// Record the current clipboard content into the (capped) history.
    pub fn capture_clipboard(&mut self) {
        if let Ok(output) = Command::new("wl-paste").arg("--no-newline").output()
            && let Ok(text) = String::from_utf8(output.stdout)
            && !text.is_empty()
        {
            self.push_clipboard(text);
        }
    }

    pub fn push_clipboard(&mut self, text: String) {
        if self.clipboard_history.iter().any(|h| *h == text) {
            return;
        }
        self.clipboard_history.push_front(text);
        while self.clipboard_history.len() > 8 {
            self.clipboard_history.pop_back();
        }
    }

    pub fn switch_layer(&mut self, layer: LayerId, qh: &QueueHandle<Self>) {
        self.current_layer = layer;
        self.sync_layout_and_redraw(qh);
    }

    pub fn sync_layout_and_redraw(&mut self, qh: &QueueHandle<Self>) {
        if self.clipboard_mode {
            let history: Vec<String> = self.clipboard_history.iter().cloned().collect();
            self.layout = KeyboardLayout::clipboard(&history, &self.suggest_engine.candidates);
        } else {
            self.layout =
                KeyboardLayout::get_layout(self.current_layer, &self.suggest_engine.candidates);
        }
        self.redraw(qh);
    }

    pub fn adapt_layout_for_content_purpose(&mut self, purpose: u32, qh: &QueueHandle<Self>) {
        if (2..=5).contains(&purpose) {
            self.switch_layer(LayerId::Symbols, qh);
        }
    }

    pub fn redraw(&mut self, _qh: &QueueHandle<Self>) {
        if !self.is_visible || !self.is_configured {
            return;
        }

        let width = self.width.max(100);
        let height = self.height.max(100);
        let stride = width * 4;

        // Rebuild the ring of reusable buffers when empty or when the surface
        // was resized. The pool is sized exactly for IN_FLIGHT_BUFFERS frames;
        // because the same buffers are reattached every frame (instead of
        // calling create_buffer), the pool freelist is never exhausted and the
        // underlying memfd stops growing.
        if self.canvas_buffers.is_empty() || self.canvas_size != (width, height) {
            let needed = (height as usize) * (stride as usize) * IN_FLIGHT_BUFFERS;
            if self.pool.len() < needed && let Err(e) = self.pool.resize(needed) {
                tracing::error!("Failed to grow SHM pool: {:?}", e);
                return;
            }
            self.canvas_buffers.clear();
            for _ in 0..IN_FLIGHT_BUFFERS {
                match self.pool.create_buffer(
                    width as i32,
                    height as i32,
                    stride as i32,
                    wl_shm::Format::Argb8888,
                ) {
                    Ok((buffer, _)) => self.canvas_buffers.push(buffer),
                    Err(e) => {
                        tracing::error!("Failed to create SHM buffer: {:?}", e);
                        self.canvas_buffers.clear();
                        return;
                    }
                }
            }
            self.canvas_size = (width, height);
        }

        // Draw into the first buffer whose slot is free (i.e. released by the
        // compositor). If all are still in flight, skip this frame — the
        // previously committed buffer remains displayed.
        for buffer in &self.canvas_buffers {
            let Some(canvas) = buffer.canvas(&mut self.pool) else {
                continue;
            };

            RenderEngine::render(
                canvas,
                width,
                height,
                &self.layout,
                &self.theme,
                self.pressed_key,
                self.swipe_offset,
            );
            // Paint swap: render the Wireframe scene through Slint when available.
            if self.slint_scene.is_none() {
                match SlintScene::new(width, height) {
                    Ok(scene) => self.slint_scene = Some(scene),
                    Err(e) => {
                        tracing::warn!("Slint renderer unavailable, using legacy renderer: {e:?}");
                    }
                }
            }
            if let Some(scene) = self.slint_scene.as_mut() {
                if scene.render(
                    &self.layout,
                    &self.theme,
                    width,
                    height,
                    self.pressed_key,
                    self.swipe_offset,
                    self.hold_preview.clone(),
                    canvas,
                ) {
                    tracing::trace!("Painted frame via Slint scene ({}x{})", width, height);
                } else {
                    tracing::warn!("Slint frame render failed, falling back to legacy renderer");
                    RenderEngine::render(
                        canvas,
                        width,
                        height,
                        &self.layout,
                        &self.theme,
                        self.pressed_key,
                        self.swipe_offset,
                    );
                }
            }

            if let Some(ref surface) = self.layer_surface
                && buffer.attach_to(surface.wl_surface()).is_ok()
            {
                surface.wl_surface().damage_buffer(0, 0, width as i32, height as i32);
                surface.commit();
            }
            break;
        }
    }

    pub fn handle_key_press(&mut self, r_idx: usize, k_idx: usize, qh: &QueueHandle<Self>) {
        let (action, secondary) = {
            let row = match self.layout.rows.get(r_idx) {
                Some(r) => r,
                None => return,
            };
            let key = match row.keys.get(k_idx) {
                Some(k) => k,
                None => return,
            };
            (key.action.clone(), key.secondary_label.clone())
        };

        tracing::debug!("Key pressed: {:?}", action);
        self.pressed_key = Some((r_idx, k_idx));
        self.press_instant = Some(Instant::now());

        if matches!(action, KeyAction::Text(_)) && secondary.is_some() {
            self.sync_layout_and_redraw(qh);
            return;
        }

        match action {
            KeyAction::Text(text) => {
                for ch in text.chars() {
                    self.suggest_engine.push_char(ch);
                }
                self.send_text(&text);
                if self.current_layer == LayerId::Upper {
                    self.current_layer = LayerId::Lower;
                }
            }
            KeyAction::Suggestion(idx) => {
                if let Some(chosen_word) = self.suggest_engine.candidates.get(idx).cloned() {
                    let preedit_len = self.suggest_engine.current_word.chars().count();
                    for _ in 0..preedit_len {
                        self.send_backspace();
                    }
                    self.send_text(&format!("{} ", chosen_word));
                    self.suggest_engine.clear();
                }
            }
            KeyAction::Backspace => {
                self.suggest_engine.pop_char();
                self.send_backspace();
            }
            KeyAction::Enter => {
                self.suggest_engine.clear();
                self.send_enter();
            }
            KeyAction::Space => {
                // Handled on release if not swiped
            }
            KeyAction::Tab => {
                self.suggest_engine.clear();
                self.send_tab();
            }
            KeyAction::Escape => {
                self.send_escape();
            }
            KeyAction::Shift => {
                let next = if self.current_layer == LayerId::Lower {
                    LayerId::Upper
                } else {
                    LayerId::Lower
                };
                self.current_layer = next;
            }
            KeyAction::SwitchLayer(layer) => {
                self.current_layer = layer;
            }
            KeyAction::Hide => {
                self.manually_shown = false;
                self.hide_keyboard(qh);
            }
            KeyAction::ArrowLeft => {
                self.send_arrow_left();
            }
            KeyAction::ArrowRight => {
                self.send_arrow_right();
            }
            KeyAction::ArrowUp => {
                self.send_arrow_up();
            }
            KeyAction::ArrowDown => {
                self.send_arrow_down();
            }
            KeyAction::Copy => {
                self.capture_clipboard();
            }
            KeyAction::Paste => {
                tracing::info!("Paste requested");
                if let Ok(output) = Command::new("wl-paste").arg("--no-newline").output()
                    && let Ok(text) = String::from_utf8(output.stdout)
                    && !text.is_empty()
                {
                    self.send_text(&text);
                    self.push_clipboard(text);
                }
            }
            KeyAction::Clipboard => {
                self.toggle_clipboard(qh);
            }
            KeyAction::ClipboardItem(idx) => {
                if let Some(text) = self.clipboard_history.get(idx).cloned() {
                    self.send_text(&text);
                }
            }
            KeyAction::Ctrl => {
                self.send_keycode(KEY_LEFTCTRL);
            }
            KeyAction::Alt => {
                self.send_keycode(KEY_LEFTALT);
            }
            KeyAction::Win => {
                self.send_keycode(KEY_LEFTMETA);
            }
            KeyAction::Home => {
                self.send_keycode(KEY_HOME);
            }
            KeyAction::End => {
                self.send_keycode(KEY_END);
            }
            KeyAction::None => {
                // Visual-only key (e.g. Mic): renders but performs no action
            }
            _ => {}
        }

        self.sync_layout_and_redraw(qh);
    }

    pub fn handle_key_release(&mut self, r_idx: usize, k_idx: usize, qh: &QueueHandle<Self>) {
        let hold_text = if let Some(row) = self.layout.rows.get(r_idx)
            && let Some(key) = row.keys.get(k_idx)
            && let KeyAction::Text(text) = &key.action
            && let Some(sec) = &key.secondary_label
        {
            let elapsed = self.press_instant.map(|t| t.elapsed()).unwrap_or_default();
            let hold = elapsed.as_millis() > 300;
            Some(if hold { sec.clone() } else { text.clone() })
        } else {
            None
        };

        let is_space_release = if let Some(row) = self.layout.rows.get(r_idx)
            && let Some(key) = row.keys.get(k_idx)
        {
            matches!(key.action, KeyAction::Space) && !self.is_space_swiping
        } else {
            false
        };

        if let Some(chosen) = hold_text {
            for ch in chosen.chars() {
                self.suggest_engine.push_char(ch);
            }
            self.send_text(&chosen);
            if self.current_layer == LayerId::Upper {
                self.current_layer = LayerId::Lower;
            }
        } else if is_space_release {
            self.suggest_engine.clear();
            self.send_space();
        }

        self.pressed_key = None;
        self.press_instant = None;
        self.hold_preview = None;
        self.space_touch_start = None;
        self.is_space_swiping = false;
        self.swipe_offset = None;
        self.sync_layout_and_redraw(qh);
    }

    pub fn update_hold_preview(&mut self, qh: &QueueHandle<Self>) {
        if let Some((r_idx, k_idx)) = self.pressed_key
            && let Some(row) = self.layout.rows.get(r_idx)
            && let Some(key) = row.keys.get(k_idx)
            && let Some(sec) = &key.secondary_label
            && let Some(instant) = self.press_instant
            && instant.elapsed().as_millis() > 300
        {
            if self.hold_preview.is_none() {
                let rects = RenderEngine::calculate_key_rects(
                    &self.layout,
                    self.width,
                    self.height,
                    &self.theme,
                );
                if let Some((rect, _)) = rects.get(r_idx).and_then(|r| r.get(k_idx)) {
                    let popup_w = rect.w * 1.22;
                    let popup_h = rect.h * 1.45;
                    let mut popup_x = rect.x + rect.w / 2.0 - popup_w / 2.0;
                    let mut popup_y = rect.y + rect.h / 2.0 - popup_h / 2.0 - 34.0;
                    popup_y = popup_y.clamp(8.0, self.height as f32 - popup_h - 8.0);
                    popup_x = popup_x.clamp(8.0, self.width as f32 - popup_w - 8.0);
                    self.hold_preview = Some(HoldPreview {
                        x: popup_x,
                        y: popup_y,
                        w: popup_w,
                        h: popup_h,
                        label: sec.clone(),
                    });
                    self.redraw(qh);
                }
            }
            return;
        }
        if self.hold_preview.is_some() {
            self.hold_preview = None;
            self.redraw(qh);
        }
    }

    pub fn handle_motion(&mut self, x: f64, _y: f64, qh: &QueueHandle<Self>) {
        if let Some((start_x, _)) = self.space_touch_start {
            let delta = x - self.space_last_x;
            let step_threshold = 18.0;

            if delta.abs() >= step_threshold {
                self.is_space_swiping = true;
                if delta < 0.0 {
                    self.send_arrow_left();
                } else {
                    self.send_arrow_right();
                }
                self.space_last_x = x;
            }

            self.swipe_offset = Some((x - start_x) as f32);
            self.redraw(qh);
        }
    }

    pub fn send_text(&mut self, text: &str) {
        let is_im_active = self.im_state.is_active.load(Ordering::SeqCst);
        if is_im_active
            && let Some(ref im) = self.input_method
        {
            im.commit_string(text.to_string());
            im.commit(self.im_state.serial);
            self.im_state.serial = self.im_state.serial.wrapping_add(1);
            return;
        }

        // Universal Virtual Keyboard Fallback for Terminal Emulators (Herdr, Foot, Kitty, etc.)
        for ch in text.chars() {
            if let Some((keycode, shift)) = char_to_keycode_and_shift(ch) {
                self.send_key_with_modifiers(keycode, shift);
            }
        }
    }

    pub fn send_backspace(&mut self) {
        // ALWAYS emit raw evdev KEY_BACKSPACE (14) via virtual keyboard!
        // This is 100% stable and prevents terminal emulator exits/crashes.
        self.send_keycode(KEY_BACKSPACE);
    }

    pub fn send_enter(&mut self) {
        self.send_keycode(KEY_ENTER);
    }

    pub fn send_tab(&mut self) {
        self.send_keycode(KEY_TAB);
    }

    pub fn send_space(&mut self) {
        let is_im_active = self.im_state.is_active.load(Ordering::SeqCst);
        if is_im_active {
            self.send_text(" ");
        } else {
            self.send_keycode(KEY_SPACE);
        }
    }

    pub fn send_arrow_left(&mut self) {
        self.send_keycode(KEY_LEFT);
    }

    pub fn send_arrow_right(&mut self) {
        self.send_keycode(KEY_RIGHT);
    }

    pub fn send_arrow_up(&mut self) {
        self.send_keycode(KEY_UP);
    }

    pub fn send_arrow_down(&mut self) {
        self.send_keycode(KEY_DOWN);
    }

    pub fn send_escape(&mut self) {
        self.send_keycode(KEY_ESC);
    }

    pub fn send_keycode(&mut self, keycode: u32) {
        self.send_key_with_modifiers(keycode, false);
    }

    pub fn send_key_with_modifiers(&mut self, keycode: u32, shift: bool) {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u32);

        if let Some(ref vk) = self.virtual_keyboard {
            if shift {
                vk.modifiers(1, 0, 0, 0); // Shift depressed
            }
            vk.key(time, keycode, wl_keyboard::KeyState::Pressed.into());
            vk.key(time, keycode, wl_keyboard::KeyState::Released.into());
            if shift {
                vk.modifiers(0, 0, 0, 0); // Reset modifiers
            }
        } else {
            tracing::warn!("No virtual keyboard bound; dropping keycode {keycode}");
        }
    }

    pub fn init_virtual_keyboard(&mut self, qh: &QueueHandle<Self>) {
        if self.virtual_keyboard.is_some() {
            return;
        }

        let (manager, seat) = match (&self.vk_manager, self.seat_state.seats().next()) {
            (Some(manager), Some(seat)) => (manager, seat),
            _ => return,
        };

        let vk = manager.create_virtual_keyboard(&seat, qh, ());
        if let Some((fd, size)) = create_keymap_fd() {
            vk.keymap(KEYMAP_FORMAT, fd.as_fd(), size);
            vk.modifiers(0, 0, 0, 0);
            tracing::info!("Virtual keyboard bound and keymap uploaded");
        } else {
            tracing::warn!("Failed to build keymap; keycodes may be unmapped");
        }
        self.virtual_keyboard = Some(vk);
    }
}

// SCTK Delegate Implementations
delegate_compositor!(WaylandState);
delegate_output!(WaylandState);
delegate_shm!(WaylandState);
delegate_layer!(WaylandState);
delegate_seat!(WaylandState);
delegate_pointer!(WaylandState);
delegate_touch!(WaylandState);
delegate_registry!(WaylandState);

impl ProvidesRegistryState for WaylandState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

impl CompositorHandler for WaylandState {
    fn scale_factor_changed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _scale: i32) {}
    fn transform_changed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _transform: wl_output::Transform) {}
    fn frame(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _time: u32) {}
    fn surface_enter(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _output: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _output: &wl_output::WlOutput) {}
}

impl OutputHandler for WaylandState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: wl_output::WlOutput) {}
    fn update_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: wl_output::WlOutput) {}
}

impl ShmHandler for WaylandState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm_state
    }
}

impl SeatHandler for WaylandState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
        self.init_virtual_keyboard(qh);
    }
    fn new_capability(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer {
            let _ = self.seat_state.get_pointer(qh, &seat);
        }
        if capability == Capability::Touch {
            let _ = self.seat_state.get_touch(qh, &seat);
        }
    }
    fn remove_capability(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat, _capability: Capability) {}
    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}
}

impl PointerHandler for WaylandState {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            match event.kind {
                PointerEventKind::Motion { .. } => {
                    let (x, y) = event.position;
                    self.handle_motion(x, y, qh);
                }
                PointerEventKind::Press { .. } => {
                    let (x, y) = event.position;
                    let rects = RenderEngine::calculate_key_rects(&self.layout, self.width, self.height, &self.theme);
                    if let Some((r_idx, k_idx, key)) = RenderEngine::hit_test(&self.layout, &rects, x, y) {
                        if matches!(key.action, KeyAction::Space) {
                            self.space_touch_start = Some((x, y));
                            self.space_last_x = x;
                            self.is_space_swiping = false;
                        }
                        self.handle_key_press(r_idx, k_idx, qh);
                    }
                }
                PointerEventKind::Release { .. } => {
                    if let Some((r_idx, k_idx)) = self.pressed_key {
                        self.handle_key_release(r_idx, k_idx, qh);
                    }
                }
                _ => {}
            }
        }
    }
}

impl TouchHandler for WaylandState {
    fn down(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        _surface: wl_surface::WlSurface,
        _id: i32,
        position: (f64, f64),
    ) {
        let (x, y) = position;
        let rects = RenderEngine::calculate_key_rects(&self.layout, self.width, self.height, &self.theme);
        if let Some((r_idx, k_idx, key)) = RenderEngine::hit_test(&self.layout, &rects, x, y) {
            if matches!(key.action, KeyAction::Space) {
                self.space_touch_start = Some((x, y));
                self.space_last_x = x;
                self.is_space_swiping = false;
            }
            self.handle_key_press(r_idx, k_idx, qh);
        }
    }

    fn up(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        _id: i32,
    ) {
        if let Some((r_idx, k_idx)) = self.pressed_key {
            self.handle_key_release(r_idx, k_idx, qh);
        }
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _time: u32,
        _id: i32,
        position: (f64, f64),
    ) {
        let (x, y) = position;
        self.handle_motion(x, y, qh);
    }

    fn cancel(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _touch: &wl_touch::WlTouch) {
        self.pressed_key = None;
        self.press_instant = None;
        self.hold_preview = None;
        self.space_touch_start = None;
        self.is_space_swiping = false;
        self.swipe_offset = None;
    }

    fn shape(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _touch: &wl_touch::WlTouch, _id: i32, _major: f64, _minor: f64) {}
    fn orientation(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _touch: &wl_touch::WlTouch, _id: i32, _orientation: f64) {}
}

/// Linux evdev keycode mapping for US QWERTY virtual keyboard fallback
pub fn char_to_keycode_and_shift(ch: char) -> Option<(u32, bool)> {
    match ch {
        'a' => Some((30, false)),
        'b' => Some((48, false)),
        'c' => Some((46, false)),
        'd' => Some((32, false)),
        'e' => Some((18, false)),
        'f' => Some((33, false)),
        'g' => Some((34, false)),
        'h' => Some((35, false)),
        'i' => Some((23, false)),
        'j' => Some((36, false)),
        'k' => Some((37, false)),
        'l' => Some((38, false)),
        'm' => Some((50, false)),
        'n' => Some((49, false)),
        'o' => Some((24, false)),
        'p' => Some((25, false)),
        'q' => Some((16, false)),
        'r' => Some((19, false)),
        's' => Some((31, false)),
        't' => Some((20, false)),
        'u' => Some((22, false)),
        'v' => Some((47, false)),
        'w' => Some((17, false)),
        'x' => Some((45, false)),
        'y' => Some((21, false)),
        'z' => Some((44, false)),

        'A' => Some((30, true)),
        'B' => Some((48, true)),
        'C' => Some((46, true)),
        'D' => Some((32, true)),
        'E' => Some((18, true)),
        'F' => Some((33, true)),
        'G' => Some((34, true)),
        'H' => Some((35, true)),
        'I' => Some((23, true)),
        'J' => Some((36, true)),
        'K' => Some((37, true)),
        'L' => Some((38, true)),
        'M' => Some((50, true)),
        'N' => Some((49, true)),
        'O' => Some((24, true)),
        'P' => Some((25, true)),
        'Q' => Some((16, true)),
        'R' => Some((19, true)),
        'S' => Some((31, true)),
        'T' => Some((20, true)),
        'U' => Some((22, true)),
        'V' => Some((47, true)),
        'W' => Some((17, true)),
        'X' => Some((45, true)),
        'Y' => Some((21, true)),
        'Z' => Some((44, true)),

        '1' => Some((2, false)),
        '2' => Some((3, false)),
        '3' => Some((4, false)),
        '4' => Some((5, false)),
        '5' => Some((6, false)),
        '6' => Some((7, false)),
        '7' => Some((8, false)),
        '8' => Some((9, false)),
        '9' => Some((10, false)),
        '0' => Some((11, false)),

        '!' => Some((2, true)),
        '@' => Some((3, true)),
        '#' => Some((4, true)),
        '$' => Some((5, true)),
        '%' => Some((6, true)),
        '^' => Some((7, true)),
        '&' => Some((8, true)),
        '*' => Some((9, true)),
        '(' => Some((10, true)),
        ')' => Some((11, true)),

        '-' => Some((12, false)),
        '_' => Some((12, true)),
        '=' => Some((13, false)),
        '+' => Some((13, true)),
        '[' => Some((26, false)),
        '{' => Some((26, true)),
        ']' => Some((27, false)),
        '}' => Some((27, true)),
        '\\' => Some((43, false)),
        '|' => Some((43, true)),
        ';' => Some((39, false)),
        ':' => Some((39, true)),
        '\'' => Some((40, false)),
        '"' => Some((40, true)),
        '`' => Some((41, false)),
        '~' => Some((41, true)),
        ',' => Some((51, false)),
        '<' => Some((51, true)),
        '.' => Some((52, false)),
        '>' => Some((52, true)),
        '/' => Some((53, false)),
        '?' => Some((53, true)),
        ' ' => Some((KEY_SPACE, false)),
        '\n' => Some((KEY_ENTER, false)),
        '\t' => Some((KEY_TAB, false)),

        _ => None,
    }
}
