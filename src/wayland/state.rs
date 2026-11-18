use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, delegate_touch,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        touch::TouchHandler,
    },
    shell::WaylandSurface,
    shell::wlr_layer::{LayerShell, LayerSurface},
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use wayland_client::{
    Connection, QueueHandle,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface, wl_touch},
};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2, zwp_input_method_v2::ZwpInputMethodV2,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

use crate::config::Config;
use crate::layout::{CustomLayoutFile, KeyboardLayout, LayerId, LayoutMode, key::KeyAction};
use crate::render::engine::{Rect, RenderEngine};
use crate::render::slint::SlintScene;
use crate::render::theme::{Color, Theme};
use crate::suggest::SuggestEngine;
use crate::suggest::swipe::{SwipeKeyGeom, SwipeRecognizer};
#[cfg(feature = "futo-swipe")]
use crate::suggest::swipe_preview::{PreviewRequest, SwipePreviewWorker};
use crate::wayland::input_method::{InputMethodState, TextInputCapabilities, TextInputSource};
use crate::wayland::layer_shell::create_layer_surface;
use crate::wayland::virtual_keyboard::{KEYMAP_FORMAT, create_keymap_fd};

// Common Linux evdev keycodes
const KEY_ESC: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_DELETE: u32 = 111;
const KEY_TAB: u32 = 15;
const KEY_ENTER: u32 = 28;
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTALT: u32 = 56;
const KEY_SPACE: u32 = 57;
const KEY_LEFTMETA: u32 = 125; // Win / Super
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_UP: u32 = 103;
const KEY_DOWN: u32 = 108;
const KEY_HOME: u32 = 102;
const KEY_END: u32 = 107;
const KEY_INSERT: u32 = 110;
const KEY_V: u32 = 47;

/// Timed finger path for one touch that started on a letter key.
#[derive(Debug, Clone)]
pub struct SwipeTrace {
    candidates: Vec<String>,
    points: Vec<(f64, f64, Instant)>,
    start_x: f64,
    start_y: f64,
    start: Instant,
    max_distance: f64,
    threshold: f64,
    last_preview_at: Option<Instant>,
}

impl SwipeTrace {
    fn new(x: f64, y: f64, now: Instant, threshold: f64) -> Self {
        Self {
            points: vec![(x, y, now)],
            candidates: Vec::new(),
            start_x: x,
            start_y: y,
            start: now,
            max_distance: 0.0,
            threshold,
            last_preview_at: None,
        }
    }

    fn push(&mut self, x: f64, y: f64, now: Instant) {
        self.max_distance = self
            .max_distance
            .max((x - self.start_x).hypot(y - self.start_y));
        const MAX_POINTS: usize = 256;
        if self.points.len() < MAX_POINTS {
            self.points.push((x, y, now));
        } else {
            // Preserve the gesture origin and current endpoint while halving
            // old samples so long swipes still decode their final direction.
            let mut compacted = Vec::with_capacity(MAX_POINTS / 2 + 1);
            if let Some(first) = self.points.first().copied() {
                compacted.push(first);
            }
            compacted.extend(
                self.points
                    .iter()
                    .skip(1)
                    .take(self.points.len().saturating_sub(2))
                    .step_by(2)
                    .copied(),
            );
            compacted.push((x, y, now));
            self.points = compacted;
        }
    }

    fn preview_due(&mut self, now: Instant) -> bool {
        if self
            .last_preview_at
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(100))
        {
            return false;
        }
        self.last_preview_at = Some(now);
        true
    }

    fn qualifies(&self) -> bool {
        self.points.len() >= 2 && self.max_distance >= self.threshold
    }
}

/// Number of SHM buffers to cycle through. The compositor holds a buffer until
/// it releases it; a small ring decouples rendering from the release round-trip
/// while capping pool growth (sctk grows the pool only when the freelist is
/// exhausted).
const IN_FLIGHT_BUFFERS: usize = 3;
const TOUCH_PRESS_FEEDBACK_DURATION: Duration = Duration::from_millis(96);

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
    pub active_exclusive_zone: Option<i32>,
    pub is_running: bool,
    pub width: u32,
    pub height: u32,
    /// True while a physical (folio) keyboard is attached. Gates auto-show in
    /// folio mode; manual show/toggle is unaffected.
    pub folio_attached: bool,
    /// Last input was via touch screen. Used for `touch_only` gating.
    pub last_input_is_touch: bool,
    pub last_input_at: Option<Instant>,
    pub last_touch_at: Option<Instant>,
    /// Normalized Y coordinate from the most recent physical touchscreen contact.
    pub last_touch_relative_y: Option<f64>,
    pub last_workspace_switch_at: Option<Instant>,
    pub last_arrow_at: Option<Instant>,

    // Reusable SHM buffers (ring of IN_FLIGHT_BUFFERS slots). Buffers are
    // reused across frames once the compositor releases them; the pool never
    // grows, unlike allocating a fresh buffer per redraw.
    pub canvas_buffers: Vec<Buffer>,
    pub canvas_size: (u32, u32),
    base_frame: Vec<u8>,
    trail_only_redraw: bool,
    trail_alpha_mask: Vec<u8>,

    // Input, Suggestions & Gesture State
    pub im_state: InputMethodState,
    pub text_input_capabilities: TextInputCapabilities,
    pub fallback_warning_emitted: bool,
    pub swipe_preview: Option<String>,
    pub fading_swipe_trail: Option<Vec<(f64, f64, Instant)>>,
    pub last_trail_frame_at: Option<Instant>,
    pub current_layer: LayerId,
    pub layout_mode: LayoutMode,
    pub custom_layout: Option<CustomLayoutFile>,
    pub layout: KeyboardLayout,
    pub key_rects: Vec<Vec<(Rect, usize)>>,
    pub theme: Theme,
    pub config: Config,
    pub suggest_engine: SuggestEngine,
    pub pressed_key: Option<(usize, usize)>,
    pub pressed_keys: Vec<(usize, usize)>,
    /// Keeps quick touch taps visible until the compositor presents a frame.
    pub touch_press_feedback: HashMap<(usize, usize), (KeyAction, Instant)>,
    pub active_touches: HashMap<i32, (usize, usize)>,
    pub pending_taps: HashMap<i32, (usize, usize)>,
    pub pending_tap_details: HashMap<i32, (KeyAction, Option<String>, Instant)>,
    pub pointer_pending_detail: Option<(KeyAction, Option<String>, Instant)>,
    /// Lazy OpenSwipe recognizer fed from `suggest_engine.dictionary`.
    pub swipe_engine: Option<SwipeRecognizer>,
    #[cfg(feature = "futo-swipe")]
    swipe_preview_worker: Option<SwipePreviewWorker>,
    #[cfg(feature = "futo-swipe")]
    swipe_preview_dictionary_revision: u64,
    #[cfg(feature = "futo-swipe")]
    swipe_preview_sequence: u64,
    #[cfg(feature = "futo-swipe")]
    pending_swipe_preview: Option<(i32, u64)>,
    /// Per-touch gesture candidates; only one trace is kept at a time.
    pub swipe_traces: HashMap<i32, SwipeTrace>,
    /// Slint headless paint scene (lazily initialized on first redraw).
    pub slint_scene: Option<SlintScene>,

    // One-shot modifiers for Hyprland Super / Ctrl / Alt (Maliit-style latch)
    pub pending_mods: u32,
    /// Physically-held modifiers (finger still down): applied to every
    /// emission while held, never auto-consumed. Released when the owning
    /// finger lifts (or via release_all_held_mods on cancel/hide/layer-switch).
    pub held_mods: u32,
    /// Held-modifier bits that have chorded at least one other key press.
    /// A held bit released with this set is a pure release (no tap-latch).
    pub held_mod_chorded: u32,
    /// When each held-modifier bit was first pressed (for tap conversion).
    pub held_mod_down_at: HashMap<u32, Instant>,
    /// Touch/pointer owners per held-modifier bit (touch id, or POINTER_TOUCH_ID).
    pub mod_owners: HashMap<u32, Vec<i32>>,
    /// True when the current Shift hold raised Lower->Upper itself.
    pub shift_hold_raised_layer: bool,
    /// True when the current Shift press qualifies as a caps double-tap.
    pub shift_tap_double: bool,
    // Spacebar Glide / Swipe Cursor Navigation
    pub space_touch_start: Option<(f64, f64)>,
    pub space_last_x: f64,
    pub is_space_swiping: bool,
    pub swipe_offset: Option<f32>,

    // Clipboard history view: when active, the key rows are replaced by the
    // clipboard grid and `layout` is built from `clipboard_history`.
    pub clipboard_mode: bool,
    pub clipboard_history: VecDeque<String>,
    /// Vertical scroll offset (px) of the clipboard card grid. The top bar
    /// and pinned bottom row stay fixed; only card rows slide.
    pub clipboard_scroll_y: f32,
    /// Active clipboard swipe: (start_x, start_y, scroll_at_start).
    pub clipboard_drag_start: Option<(f64, f64, f32)>,
    /// True once the drag exceeds the tap threshold (paste is suppressed).
    pub clipboard_dragging: bool,

    // Hold-to-type secondary (sub) characters: press time for the current key.
    pub press_instant: Option<Instant>,
    pub hold_preview: Option<HoldPreview>,

    // Key repeat (e.g. holding backspace to continuously delete)
    pub repeat_key: Option<KeyAction>,
    pub repeat_start: Option<Instant>,
    pub last_repeat_at: Option<Instant>,

    // Double-tap Shift CapsLock
    pub caps_lock: bool,
    pub last_shift_press: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct HoldPreview {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub label: String,
}

/// Compute effective keyboard height: custom layout height when present, otherwise general height.
pub fn compute_effective_keyboard_height(custom_height: Option<u32>, general_height: u32) -> u32 {
    custom_height.unwrap_or(general_height)
}

impl WaylandState {
    /// Returns the effective keyboard height: custom_layout.height when present,
    /// otherwise config.general.height.
    pub fn effective_keyboard_height(&self) -> u32 {
        compute_effective_keyboard_height(
            self.custom_layout.as_ref().and_then(|c| c.height),
            self.config.general.height,
        )
    }

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
        let layout_mode =
            LayoutMode::parse_mode(&config.general.layout_mode).unwrap_or(LayoutMode::Desktop);
        let custom_layout = config
            .general
            .layout_file
            .as_deref()
            .or(Some(config.general.layout_mode.as_str()))
            .and_then(|f| CustomLayoutFile::load_from_name_or_path(f).ok());
        let height = compute_effective_keyboard_height(
            custom_layout.as_ref().and_then(|c| c.height),
            config.general.height,
        );
        let pool = SlotPool::new((1000 * height.max(1) * 4) as usize, shm)
            .expect("Failed to create SHM slot pool");
        let theme = Theme::from(&config.theme);
        let layout = if let Some(ref custom) = custom_layout {
            custom.to_keyboard_layout(LayerId::Lower, &[], false)
        } else {
            KeyboardLayout::get_layout_for_mode(LayerId::Lower, &[], false, layout_mode)
        };
        let key_rects = RenderEngine::calculate_key_rects(&layout, 1000, height, &theme);
        let suggest_engine = SuggestEngine::new();
        #[cfg(feature = "futo-swipe")]
        let swipe_preview_dictionary_revision = suggest_engine.dictionary.swipe_lexicon_revision();
        #[cfg(feature = "futo-swipe")]
        let swipe_preview_initial_dictionary = suggest_engine.dictionary.clone();

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
            active_exclusive_zone: None,
            is_running: true,
            width: 1000,
            height,
            folio_attached: crate::folio::physical_keyboard_attached(),
            last_input_is_touch: false,
            last_input_at: None,
            last_touch_at: None,
            last_touch_relative_y: None,
            last_workspace_switch_at: None,
            last_arrow_at: None,
            pending_mods: 0,
            held_mods: 0,
            held_mod_chorded: 0,
            held_mod_down_at: HashMap::new(),
            mod_owners: HashMap::new(),
            shift_hold_raised_layer: false,
            shift_tap_double: false,
            canvas_buffers: Vec::new(),
            canvas_size: (0, 0),
            base_frame: Vec::new(),
            trail_only_redraw: false,
            trail_alpha_mask: Vec::new(),
            im_state: InputMethodState::default(),
            text_input_capabilities: TextInputCapabilities::default(),
            fallback_warning_emitted: false,
            current_layer: LayerId::Lower,
            layout_mode,
            custom_layout,
            layout,
            key_rects,
            theme,
            config,
            suggest_engine,
            pressed_key: None,
            pressed_keys: Vec::new(),
            touch_press_feedback: HashMap::new(),
            active_touches: HashMap::new(),
            pending_taps: HashMap::new(),
            pending_tap_details: HashMap::new(),
            pointer_pending_detail: None,
            swipe_engine: None,
            #[cfg(feature = "futo-swipe")]
            swipe_preview_worker: SwipePreviewWorker::new(swipe_preview_initial_dictionary),
            #[cfg(feature = "futo-swipe")]
            swipe_preview_dictionary_revision,
            #[cfg(feature = "futo-swipe")]
            swipe_preview_sequence: 0,
            #[cfg(feature = "futo-swipe")]
            pending_swipe_preview: None,
            swipe_traces: HashMap::new(),
            slint_scene: None,
            space_touch_start: None,
            swipe_preview: None,
            fading_swipe_trail: None,
            last_trail_frame_at: None,
            space_last_x: 0.0,
            is_space_swiping: false,
            swipe_offset: None,
            clipboard_mode: false,
            clipboard_history: Self::load_clipboard_history(),
            clipboard_scroll_y: 0.0,
            clipboard_drag_start: None,
            clipboard_dragging: false,
            press_instant: None,
            hold_preview: None,
            repeat_key: None,
            repeat_start: None,
            last_repeat_at: None,
            caps_lock: false,
            last_shift_press: None,
        }
    }

    pub fn init_surface(&mut self, qh: &QueueHandle<Self>) {
        if self.layer_surface.is_none() {
            let surface = create_layer_surface(self, qh);
            self.layer_surface = Some(surface);
        }
    }

    /// Return the source currently used for text exchange with the focused app.
    /// Input-method v2 is the only source that provides readable surrounding
    /// text; the virtual keyboard remains a write-only fallback.
    pub fn active_text_input_source(&self) -> TextInputSource {
        if self.input_method.is_some() && self.im_state.is_active.load(Ordering::SeqCst) {
            TextInputSource::InputMethodV2
        } else if self.virtual_keyboard.is_some() {
            TextInputSource::VirtualKeyboard
        } else {
            TextInputSource::Unavailable
        }
    }

    pub fn log_text_input_capabilities(&self) {
        let input_method_bound = self.input_method.is_some();
        let input_method_active = self.im_state.is_active.load(Ordering::SeqCst);
        tracing::info!(
            "Text input capabilities: input_method_v2={} text_input_v3={} virtual_keyboard_v1={} surrounding_text_readback={} input_method_bound={} input_method_active={} active_source={}",
            self.text_input_capabilities.input_method_v2,
            self.text_input_capabilities.text_input_v3,
            self.text_input_capabilities.virtual_keyboard_v1,
            self.config.behavior.surrounding_text_readback,
            input_method_bound,
            input_method_active,
            self.active_text_input_source()
                .label_with_readback(self.config.behavior.surrounding_text_readback),
        );
        if !self.config.behavior.surrounding_text_readback {
            tracing::info!(
                "surrounding_text_readback=false: host surrounding text readback is disabled (write-only mode)"
            );
        }
        if self.text_input_capabilities.input_method_v2
            && input_method_bound
            && !input_method_active
        {
            tracing::info!(
                "Input-method-v2 is ready; waiting for a focused text field to activate it"
            );
        } else if !input_method_bound && self.text_input_capabilities.input_method_v2 {
            tracing::warn!(
                "Input-method-v2 is advertised but unavailable on this seat; using virtual-keyboard fallback"
            );
        } else if !self.text_input_capabilities.input_method_v2 {
            tracing::warn!(
                "Text readback is unavailable; using virtual-keyboard fallback (write-only)"
            );
        }
    }

    /// Perform best-effort cursor-position inference for auto-show.
    ///
    /// If the sampled cursor Y coordinate lies in the upper half of the monitor
    /// containing that point, HyprOsk is presented as a floating overlay (exclusive_zone=0)
    /// to avoid resizing/shifting windows when the input field is far above the keyboard.
    ///
    /// If in the lower half, HyprOsk docks and reserves space (exclusive_zone=height)
    /// so the compositor tiles/shifts the window upward, preventing the keyboard from
    /// obscuring the input field.
    ///
    /// If the cursor position cannot be obtained or validated (e.g. non-Hyprland compositor,
    /// socket timeout, malformed output), falls back safely to the user's configured exclusivity.
    pub fn infer_auto_placement_exclusivity(&self) -> (bool, i32) {
        let eff_height = self.effective_keyboard_height();

        // Touchscreens may not update Hyprland's pointer position. Use the raw
        // touchscreen Y coordinate when available instead of the stale cursor.
        let is_recent_touch = self
            .last_touch_at
            .is_some_and(|at| at.elapsed().as_millis() < 1500);

        if is_recent_touch {
            if let Some(rel_y) = self.last_touch_relative_y {
                let is_upper = rel_y < 0.5;
                let exclusive = !is_upper;
                let zone = if exclusive { eff_height as i32 } else { 0 };
                tracing::info!(
                    "Auto-placement: touchscreen rel_y={:.2} -> {} -> {} (exclusive_zone={})",
                    rel_y,
                    if is_upper { "upper half" } else { "lower half" },
                    if exclusive {
                        "docked"
                    } else {
                        "floating overlay"
                    },
                    zone
                );
                return (exclusive, zone);
            }

            let (exclusive, zone) = crate::hyprland::resolve_placement_exclusivity(
                None,
                self.config.general.exclusive_zone,
                eff_height,
            );
            tracing::info!(
                "Auto-placement: touchscreen position unavailable; using configured exclusive_zone={}",
                zone
            );
            return (exclusive, zone);
        }

        // 2. Pointer-driven activation: sample Hyprland cursor position
        let infer_res = crate::hyprland::infer_cursor_position_placement();
        if let Some(ref res) = infer_res {
            // Explicitly treat the known unpositioned compositor center sentinel
            // (e.g. 720.0, 480.0 on 1440x960) as unavailable data so placement falls
            // back to configured exclusivity instead of guessing docked from the default center.
            let (mx, my, mw, mh) = res.monitor_rect;
            let center_x = mx + mw / 2.0;
            let center_y = my + mh / 2.0;
            let is_unpositioned_sentinel =
                (res.cursor.0 - center_x).abs() <= 1.0 && (res.cursor.1 - center_y).abs() <= 1.0;

            if is_unpositioned_sentinel {
                let (exclusive, zone) = crate::hyprland::resolve_placement_exclusivity(
                    None,
                    self.config.general.exclusive_zone,
                    eff_height,
                );
                tracing::info!(
                    "Auto-placement: cursor=({:.1}, {:.1}) is at monitor '{}' unpositioned center sentinel; falling back to configured exclusive_zone={}",
                    res.cursor.0,
                    res.cursor.1,
                    res.monitor_name,
                    zone
                );
                return (exclusive, zone);
            }

            let is_upper = res.position == crate::hyprland::CursorVerticalPosition::UpperHalf;
            let (exclusive, zone) = crate::hyprland::resolve_placement_exclusivity(
                Some(res),
                self.config.general.exclusive_zone,
                eff_height,
            );
            tracing::info!(
                "Auto-placement best-effort inference: cursor=({:.1}, {:.1}) on monitor '{}' ({:.0}x{:.0} at {:.0},{:.0}), rel_y={:.2} -> {} -> {} (exclusive_zone={})",
                res.cursor.0,
                res.cursor.1,
                res.monitor_name,
                res.monitor_rect.2,
                res.monitor_rect.3,
                res.monitor_rect.0,
                res.monitor_rect.1,
                res.rel_y,
                if is_upper { "upper half" } else { "lower half" },
                if exclusive {
                    "docked"
                } else {
                    "floating overlay"
                },
                zone
            );
            (exclusive, zone)
        } else {
            let (exclusive, zone) = crate::hyprland::resolve_placement_exclusivity(
                None,
                self.config.general.exclusive_zone,
                eff_height,
            );
            tracing::info!(
                "Auto-placement: best-effort cursor inference unavailable; falling back to configured exclusive_zone={}",
                zone
            );
            (exclusive, zone)
        }
    }

    /// Automatically show the keyboard on input-method-v2 Activate, using best-effort
    /// cursor-position inference to dynamically choose between floating overlay and docked.
    pub fn show_keyboard_with_auto_placement(&mut self, qh: &QueueHandle<Self>) {
        let (_exclusive, zone) = self.infer_auto_placement_exclusivity();
        self.show_keyboard_with_zone(qh, zone, false);
    }

    /// Manually show the keyboard. Respects the configured `exclusive_zone` without override.
    pub fn show_keyboard(&mut self, qh: &QueueHandle<Self>) {
        let eff_height = self.effective_keyboard_height();
        let zone = if self.config.general.exclusive_zone {
            eff_height as i32
        } else {
            0
        };
        self.show_keyboard_with_zone(qh, zone, true);
    }

    /// Show the keyboard with an explicit exclusive zone and manual flag.
    pub fn show_keyboard_with_zone(&mut self, qh: &QueueHandle<Self>, zone: i32, manual: bool) {
        if self.is_visible {
            // Focus transition: if already visible and not manually shown, update exclusivity
            if !self.manually_shown && !manual && self.active_exclusive_zone != Some(zone) {
                self.active_exclusive_zone = Some(zone);
                if let Some(ref surface) = self.layer_surface {
                    surface.set_exclusive_zone(zone);
                    surface.commit();
                }
            }
            return;
        }
        tracing::info!(
            "Showing HyprOsk on-screen keyboard (exclusive_zone={}, manual={})",
            zone,
            manual
        );
        self.is_visible = true;
        self.manually_shown = manual;
        self.active_exclusive_zone = Some(zone);
        if self.layer_surface.is_none() {
            self.init_surface(qh);
        } else {
            self.sync_surface_geometry();
        }

        if let Some(ref text) = self.im_state.surrounding_text {
            let cursor = self.im_state.cursor_pos.min(self.im_state.anchor_pos);
            self.suggest_engine.sync_with_text(text, cursor as usize);
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
        self.swipe_traces.clear();
        #[cfg(feature = "futo-swipe")]
        self.invalidate_swipe_preview();
        self.swipe_preview = None;
        self.fading_swipe_trail = None;
        self.is_visible = false;
        self.is_configured = false;
        self.manually_shown = false;
        self.active_exclusive_zone = None;
        if let Some(ref surface) = self.layer_surface {
            surface.set_exclusive_zone(0);
            surface.wl_surface().attach(None, 0, 0);
            surface.commit();
        }
        self.clipboard_mode = false;
        self.release_all_held_mods();
        self.suggest_engine.dictionary.flush_user_dict();
    }

    pub fn toggle_keyboard(&mut self, qh: &QueueHandle<Self>) {
        if self.is_visible {
            self.hide_keyboard(qh);
        } else {
            self.show_keyboard(qh);
        }
    }

    pub fn set_layout_mode(&mut self, mode: LayoutMode, qh: &QueueHandle<Self>) {
        if self.layout_mode != mode {
            self.layout_mode = mode;
            let mode_str = match mode {
                LayoutMode::Mobile => "mobile",
                LayoutMode::Desktop => "desktop",
            };
            if let Ok(custom) = CustomLayoutFile::load_from_name_or_path(mode_str) {
                self.custom_layout = Some(custom);
            } else if self.config.general.layout_file.is_none() {
                self.custom_layout = None;
            }
            self.sync_surface_geometry();
            tracing::info!("HyprOsk layout mode set to: {:?}", mode);
            if self.is_visible && self.is_configured {
                self.sync_layout_and_redraw(qh);
            }
        }
    }

    pub fn show_keyboard_with_mode(&mut self, qh: &QueueHandle<Self>, mode: LayoutMode) {
        let mode_changed = self.layout_mode != mode;
        if mode_changed {
            self.layout_mode = mode;
            let mode_str = match mode {
                LayoutMode::Mobile => "mobile",
                LayoutMode::Desktop => "desktop",
            };
            if let Ok(custom) = CustomLayoutFile::load_from_name_or_path(mode_str) {
                self.custom_layout = Some(custom);
            } else if self.config.general.layout_file.is_none() {
                self.custom_layout = None;
            }
            self.sync_surface_geometry();
        }
        if self.is_visible {
            if mode_changed && self.is_configured {
                self.sync_layout_and_redraw(qh);
            }
        } else {
            self.show_keyboard(qh);
        }
    }

    pub fn toggle_keyboard_with_mode(&mut self, qh: &QueueHandle<Self>, mode: LayoutMode) {
        let mode_changed = self.layout_mode != mode;
        if mode_changed {
            self.layout_mode = mode;
            let mode_str = match mode {
                LayoutMode::Mobile => "mobile",
                LayoutMode::Desktop => "desktop",
            };
            if let Ok(custom) = CustomLayoutFile::load_from_name_or_path(mode_str) {
                self.custom_layout = Some(custom);
            } else if self.config.general.layout_file.is_none() {
                self.custom_layout = None;
            }
            self.sync_surface_geometry();
        }
        if self.is_visible {
            if mode_changed {
                if self.is_configured {
                    self.sync_layout_and_redraw(qh);
                }
            } else {
                self.hide_keyboard(qh);
            }
        } else {
            self.show_keyboard(qh);
        }
    }

    pub fn show_keyboard_with_exclusivity(&mut self, qh: &QueueHandle<Self>, exclusive: bool) {
        self.config.general.exclusive_zone = exclusive;
        self.active_exclusive_zone = None;
        if self.is_visible {
            if let Some(ref surface) = self.layer_surface {
                let zone = if exclusive {
                    self.effective_keyboard_height() as i32
                } else {
                    0
                };
                surface.set_exclusive_zone(zone);
                surface.commit();
            }
        } else {
            self.show_keyboard(qh);
        }
    }

    pub fn toggle_keyboard_with_exclusivity(&mut self, qh: &QueueHandle<Self>, exclusive: bool) {
        if self.is_visible {
            if self.config.general.exclusive_zone != exclusive {
                self.config.general.exclusive_zone = exclusive;
                self.active_exclusive_zone = None;
                if let Some(ref surface) = self.layer_surface {
                    let zone = if exclusive {
                        self.effective_keyboard_height() as i32
                    } else {
                        0
                    };
                    surface.set_exclusive_zone(zone);
                    surface.commit();
                }
            } else {
                self.hide_keyboard(qh);
            }
        } else {
            self.show_keyboard_with_exclusivity(qh, exclusive);
        }
    }

    pub fn set_exclusivity(&mut self, _qh: &QueueHandle<Self>, exclusive: bool) {
        self.config.general.exclusive_zone = exclusive;
        self.active_exclusive_zone = None;
        tracing::info!(
            "HyprOsk exclusivity set: exclusive_zone = {}",
            self.config.general.exclusive_zone
        );
        if self.is_visible {
            if let Some(ref surface) = self.layer_surface {
                let zone = if self.config.general.exclusive_zone {
                    self.effective_keyboard_height() as i32
                } else {
                    0
                };
                surface.set_exclusive_zone(zone);
                surface.commit();
            }
        }
    }

    pub fn toggle_exclusivity(&mut self, _qh: &QueueHandle<Self>) {
        self.config.general.exclusive_zone = !self.config.general.exclusive_zone;
        self.active_exclusive_zone = None;
        tracing::info!(
            "HyprOsk exclusivity toggled: exclusive_zone = {}",
            self.config.general.exclusive_zone
        );
        if self.is_visible {
            if let Some(ref surface) = self.layer_surface {
                let zone = if self.config.general.exclusive_zone {
                    self.effective_keyboard_height() as i32
                } else {
                    0
                };
                surface.set_exclusive_zone(zone);
                surface.commit();
            }
        }
    }

    pub fn reload_config(&mut self, qh: &QueueHandle<Self>) {
        tracing::info!("Reloading HyprOsk configuration");
        let new_config = Config::load_or_create(None);
        self.theme = Theme::from(&new_config.theme);
        self.layout_mode =
            LayoutMode::parse_mode(&new_config.general.layout_mode).unwrap_or(LayoutMode::Desktop);
        self.custom_layout = new_config
            .general
            .layout_file
            .as_deref()
            .or(Some(new_config.general.layout_mode.as_str()))
            .and_then(|f| CustomLayoutFile::load_from_name_or_path(f).ok());
        self.config = new_config;
        self.height = self.effective_keyboard_height();
        self.active_exclusive_zone = None;
        self.sync_surface_geometry();

        if self.is_visible && self.is_configured {
            self.sync_layout_and_redraw(qh);
        }
    }

    pub fn sync_surface_geometry(&mut self) {
        if !self.is_visible {
            return;
        }
        if let Some(ref surface) = self.layer_surface {
            use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity};

            let target_h = self.effective_keyboard_height();
            let (target_w, margin_b, margin_h) = if let Some(ref custom) = self.custom_layout {
                let w = custom.width.unwrap_or(0);
                let mb = custom
                    .margin_bottom
                    .unwrap_or(self.config.general.margin_bottom);
                let mh = if custom.width.is_some() {
                    0
                } else {
                    self.config.general.margin_horizontal
                };
                (w, mb, mh)
            } else {
                (
                    0,
                    self.config.general.margin_bottom,
                    self.config.general.margin_horizontal,
                )
            };

            self.height = target_h;
            if target_w > 0 {
                surface.set_anchor(Anchor::BOTTOM);
                surface.set_size(target_w, target_h);
                surface.set_margin(0, 0, margin_b, 0);
            } else {
                surface.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
                surface.set_size(0, target_h);
                surface.set_margin(0, margin_h, margin_b, margin_h);
            }

            let zone = match self.active_exclusive_zone {
                Some(0) => 0,
                Some(_) => {
                    let new_zone = self.height as i32;
                    self.active_exclusive_zone = Some(new_zone);
                    new_zone
                }
                None => {
                    if self.config.general.exclusive_zone {
                        self.height as i32
                    } else {
                        0
                    }
                }
            };
            surface.set_exclusive_zone(zone);
            surface.set_keyboard_interactivity(KeyboardInteractivity::None);
            surface.commit();
        }
    }

    pub fn load_layout(
        &mut self,
        name_or_path: &str,
        qh: &QueueHandle<Self>,
    ) -> anyhow::Result<()> {
        let custom = CustomLayoutFile::load_from_name_or_path(name_or_path)?;
        if let Some(mode) = LayoutMode::parse_mode(&custom.mode) {
            self.layout_mode = mode;
        }
        self.custom_layout = Some(custom);
        self.sync_surface_geometry();
        if self.is_visible && self.is_configured {
            self.sync_layout_and_redraw(qh);
        }
        Ok(())
    }

    pub fn note_input(&mut self, is_touch: bool) {
        self.last_input_is_touch = is_touch;
        self.last_input_at = Some(Instant::now());
        if is_touch {
            self.last_touch_at = Some(Instant::now());
            self.last_touch_relative_y = None;
        }
    }

    pub fn note_touch(&mut self, event: crate::touch::TouchEvent) {
        self.last_input_is_touch = true;
        self.last_input_at = Some(event.at);
        self.last_touch_at = Some(event.at);
        self.last_touch_relative_y = event.relative_y;
    }

    pub fn note_workspace_switch(&mut self) {
        self.last_workspace_switch_at = Some(Instant::now());
    }

    /// Whether automatic activation (text-field focus) may show the keyboard.
    ///
    /// In folio mode this is denied while a physical keyboard is attached;
    /// in touch_only mode only touch triggers are allowed.
    /// Manual shows via `hyprosk show` / `toggle` are not affected.
    pub fn allow_auto_show(&mut self) -> bool {
        if let Some(ws_at) = self.last_workspace_switch_at
            && ws_at.elapsed().as_millis() < 900
        {
            tracing::debug!("suppressing auto-show: workspace switch <900ms ago (ghostty fix)");
            return false;
        }
        if self.config.behavior.folio_mode {
            // The poller debounces attach transitions. Refresh a negative result
            // at activation so an already connected folio cannot lose a race.
            if !self.folio_attached && crate::folio::physical_keyboard_attached() {
                self.folio_attached = true;
                tracing::info!("Folio keyboard detected at auto-show; suppressing OSK");
            }
            if self.folio_attached {
                tracing::debug!("Folio keyboard attached, suppressing auto-show");
                return false;
            }
        }
        if self.config.behavior.touch_only {
            let recent_touch = self
                .last_touch_at
                .or(self.last_input_at.filter(|_| self.last_input_is_touch))
                .is_some_and(|at| at.elapsed().as_millis() < 1500);
            if !recent_touch {
                tracing::debug!("touch_only enabled, suppressing auto-show (no recent touch)");
                return false;
            }
        }
        true
    }

    const MOD_SHIFT: u32 = 1;
    const MOD_CTRL: u32 = 4;
    const MOD_ALT: u32 = 8;
    const MOD_SUPER: u32 = 64;
    /// Owner id used for pointer-driven modifier holds (touch ids are >= 0 in practice).
    const POINTER_TOUCH_ID: i32 = i32::MIN;
    /// Max hold duration that still converts to a one-shot tap on release.
    const MOD_TAP_MAX_MS: u128 = 1000;

    fn sync_mods(&self) {
        if let Some(ref vk) = self.virtual_keyboard {
            vk.modifiers(self.pending_mods | self.held_mods, 0, 0, 0);
        }
    }

    fn vk_time_now() -> u32 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u32)
    }

    fn vk_mod_keycode(mask: u32) -> Option<u32> {
        match mask {
            Self::MOD_SHIFT => Some(KEY_LEFTSHIFT),
            Self::MOD_CTRL => Some(KEY_LEFTCTRL),
            Self::MOD_ALT => Some(KEY_LEFTALT),
            Self::MOD_SUPER => Some(KEY_LEFTMETA),
            _ => None,
        }
    }

    fn mod_mask_for_action(action: &KeyAction) -> u32 {
        match action {
            KeyAction::Shift => Self::MOD_SHIFT,
            KeyAction::Ctrl => Self::MOD_CTRL,
            KeyAction::Alt => Self::MOD_ALT,
            KeyAction::Win => Self::MOD_SUPER,
            _ => 0,
        }
    }

    /// Emit a physical press for a held modifier (no-op without a bound vk).
    fn vk_press_mod(&self, mask: u32) {
        if let (Some(vk), Some(keycode)) = (&self.virtual_keyboard, Self::vk_mod_keycode(mask)) {
            vk.key(
                Self::vk_time_now(),
                keycode,
                wl_keyboard::KeyState::Pressed.into(),
            );
        }
    }

    /// Emit a physical release for a held modifier (no-op without a bound vk).
    fn vk_release_mod(&self, mask: u32) {
        if let (Some(vk), Some(keycode)) = (&self.virtual_keyboard, Self::vk_mod_keycode(mask)) {
            vk.key(
                Self::vk_time_now(),
                keycode,
                wl_keyboard::KeyState::Released.into(),
            );
        }
    }

    /// Modifier bits currently owned by the given touch/pointer id.
    fn mods_owned_by(&self, owner: i32) -> Vec<u32> {
        self.mod_owners
            .iter()
            .filter_map(|(mask, owners)| owners.contains(&owner).then_some(*mask))
            .collect()
    }

    /// Register a modifier press: track as physically held, press the
    /// hardware keycode immediately, and sync compositor modifier state.
    fn mod_down(&mut self, mask: u32, owner: i32, qh: &QueueHandle<Self>) {
        let now = Instant::now();
        let owners = self.mod_owners.entry(mask).or_default();
        let first_owner = owners.is_empty();
        if !owners.contains(&owner) {
            owners.push(owner);
        }
        self.held_mod_down_at.entry(mask).or_insert(now);
        self.held_mods |= mask;
        if first_owner {
            self.vk_press_mod(mask);
        }
        self.sync_mods();
        tracing::info!("mod down {:b} held now {:b}", mask, self.held_mods);
        self.redraw(qh);
    }

    /// Release a held modifier. A hold that chorded other keys is a pure
    /// release; a quick chordless hold converts to a one-shot tap latch
    /// (Shift keeps its double-tap CapsLock behavior).
    fn mod_up(&mut self, mask: u32, owner: i32, qh: &QueueHandle<Self>) {
        if let Some(owners) = self.mod_owners.get_mut(&mask) {
            owners.retain(|&o| o != owner);
        }
        let still_owned = self.mod_owners.get(&mask).is_some_and(|o| !o.is_empty());
        if still_owned {
            // Another finger still holds this modifier.
            self.sync_mods();
            self.redraw(qh);
            return;
        }
        self.mod_owners.remove(&mask);
        if mask == Self::MOD_SHIFT {
            self.shift_up(qh);
            return;
        }
        let chorded = self.held_mod_chorded & mask != 0;
        let quick = self
            .held_mod_down_at
            .get(&mask)
            .is_some_and(|t| t.elapsed().as_millis() <= Self::MOD_TAP_MAX_MS);
        self.held_mods &= !mask;
        self.held_mod_down_at.remove(&mask);
        self.held_mod_chorded &= !mask;
        self.vk_release_mod(mask);
        if !chorded && quick {
            // Plain tap: classic sticky one-shot latch (toggles).
            self.toggle_mod(mask, qh);
        } else {
            self.sync_mods();
            self.redraw(qh);
        }
    }

    /// Shift-specific release: chorded holds revert a self-raised Upper
    /// layer, quick taps keep the classic toggle / double-tap CapsLock.
    fn shift_up(&mut self, qh: &QueueHandle<Self>) {
        let chorded = self.held_mod_chorded & Self::MOD_SHIFT != 0;
        let quick = self
            .held_mod_down_at
            .get(&Self::MOD_SHIFT)
            .is_some_and(|t| t.elapsed().as_millis() <= Self::MOD_TAP_MAX_MS);
        let raised = self.shift_hold_raised_layer;
        self.held_mods &= !Self::MOD_SHIFT;
        self.held_mod_down_at.remove(&Self::MOD_SHIFT);
        self.held_mod_chorded &= !Self::MOD_SHIFT;
        self.shift_hold_raised_layer = false;
        let double = self.shift_tap_double;
        self.shift_tap_double = false;
        self.vk_release_mod(Self::MOD_SHIFT);
        if chorded {
            // Held Shift + tapped keys: revert a self-raised layer, no latch.
            if raised && !self.caps_lock {
                self.current_layer = LayerId::Lower;
                self.pending_mods &= !Self::MOD_SHIFT;
            }
            self.sync_mods();
            self.sync_layout_and_redraw(qh);
        } else if !quick {
            // Slow hold without chords: silent release, revert self-raised layer.
            if raised && !self.caps_lock {
                self.current_layer = LayerId::Lower;
                self.pending_mods &= !Self::MOD_SHIFT;
            }
            self.sync_mods();
            self.sync_layout_and_redraw(qh);
        } else if self.caps_lock {
            self.caps_lock = false;
            self.current_layer = LayerId::Lower;
            self.pending_mods &= !Self::MOD_SHIFT;
            self.last_shift_press = None;
            self.sync_mods();
            tracing::info!("caps lock OFF");
            self.sync_layout_and_redraw(qh);
        } else if raised {
            // Tap from Lower: DOWN already raised to Upper -> keep latched one-shot.
            self.pending_mods |= Self::MOD_SHIFT;
            self.sync_mods();
            self.sync_layout_and_redraw(qh);
        } else if double {
            self.caps_lock = true;
            self.current_layer = LayerId::Upper;
            self.pending_mods |= Self::MOD_SHIFT;
            self.last_shift_press = None;
            self.sync_mods();
            tracing::info!("caps lock ON via double tap");
            self.sync_layout_and_redraw(qh);
        } else {
            // Tap while Upper was already latched: unlatch back to Lower.
            self.current_layer = LayerId::Lower;
            self.pending_mods &= !Self::MOD_SHIFT;
            self.sync_mods();
            self.sync_layout_and_redraw(qh);
        }
    }

    /// Register a Shift press: hold-track it and raise Lower->Upper immediately
    /// so chorded letters preview/emission see the Upper layer.
    fn shift_down(&mut self, owner: i32, qh: &QueueHandle<Self>) {
        let now = Instant::now();
        self.shift_tap_double = self
            .last_shift_press
            .is_some_and(|p| p.elapsed().as_millis() <= 350);
        self.last_shift_press = Some(now);
        let owners = self.mod_owners.entry(Self::MOD_SHIFT).or_default();
        let first_owner = owners.is_empty();
        if !owners.contains(&owner) {
            owners.push(owner);
        }
        self.held_mod_down_at.entry(Self::MOD_SHIFT).or_insert(now);
        self.held_mods |= Self::MOD_SHIFT;
        if first_owner {
            self.vk_press_mod(Self::MOD_SHIFT);
        }
        if !self.caps_lock && self.current_layer == LayerId::Lower {
            self.current_layer = LayerId::Upper;
            self.pending_mods |= Self::MOD_SHIFT;
            self.shift_hold_raised_layer = true;
        } else {
            self.shift_hold_raised_layer = false;
        }
        self.sync_mods();
        tracing::info!("shift down held now {:b}", self.held_mods);
        self.sync_layout_and_redraw(qh);
    }

    /// Physically release every held modifier and reset compositor modifier
    /// state to pending-only. Prevents stuck keys on cancel/hide/layer-switch.
    fn release_all_held_mods(&mut self) {
        if self.held_mods == 0 {
            return;
        }
        for mask in [
            Self::MOD_SHIFT,
            Self::MOD_CTRL,
            Self::MOD_ALT,
            Self::MOD_SUPER,
        ] {
            if self.held_mods & mask != 0 {
                self.vk_release_mod(mask);
            }
        }
        self.held_mods = 0;
        self.held_mod_chorded = 0;
        self.held_mod_down_at.clear();
        self.mod_owners.clear();
        self.shift_hold_raised_layer = false;
        self.shift_tap_double = false;
        self.sync_mods();
        tracing::info!("released all held modifiers");
    }

    fn toggle_mod(&mut self, mask: u32, qh: &QueueHandle<Self>) {
        if self.pending_mods & mask != 0 {
            self.pending_mods &= !mask;
        } else {
            self.pending_mods |= mask;
        }
        self.sync_mods();
        tracing::info!("pending mods now {:b}", self.pending_mods);
        self.redraw(qh);
    }

    fn latched_positions(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let latched = self.pending_mods | self.held_mods;
        if latched == 0 {
            return out;
        }
        for (r_idx, row) in self.layout.rows.iter().enumerate() {
            for (k_idx, key) in row.keys.iter().enumerate() {
                let mask = match key.action {
                    KeyAction::Shift => Self::MOD_SHIFT,
                    KeyAction::Ctrl => Self::MOD_CTRL,
                    KeyAction::Alt => Self::MOD_ALT,
                    KeyAction::Win => Self::MOD_SUPER,
                    _ => 0,
                };
                if mask != 0 && (latched & mask) != 0 {
                    out.push((r_idx, k_idx));
                }
            }
        }
        out
    }

    /// React to a folio attach/detach poll result.
    ///
    /// Detaching has no immediate effect (auto-show resumes on next focus);
    /// attaching hides a keyboard that was only up because no keyboard was
    /// present, leaving manually-shown keyboards alone so debug sessions and
    /// the Folio test runs survive.
    pub fn on_folio_change(&mut self, attached: bool, qh: &QueueHandle<Self>) {
        self.folio_attached = attached;
        tracing::info!(
            "Folio keyboard {} -> auto-show {}",
            if attached { "attached" } else { "detached" },
            if self.allow_auto_show() {
                "enabled"
            } else {
                "suppressed"
            }
        );
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
        // Always reset swipe and pressed state when (re)opening or closing the view.
        self.pressed_keys.clear();
        self.pressed_key = None;
        self.press_instant = None;
        self.hold_preview = None;
        self.clipboard_scroll_y = 0.0;
        self.clipboard_drag_start = None;
        self.clipboard_dragging = false;
        self.sync_layout_and_redraw(qh);
    }

    /// Maximum vertical scroll (px) for the clipboard card grid.
    pub fn clipboard_max_scroll(&self) -> f32 {
        if !self.clipboard_mode {
            return 0.0;
        }
        RenderEngine::clipboard_scroll_metrics(
            &self.layout,
            self.width,
            self.height,
            &self.theme,
        )
        .map(|(_, _, _, max)| max)
        .unwrap_or(0.0)
    }

    /// 0.0..=1.0 scroll progress for the floating scrollbar pill.
    pub fn scrollbar_ratio(&self) -> f32 {
        let max = self.clipboard_max_scroll();
        if max <= 0.0 {
            0.0
        } else {
            (self.clipboard_scroll_y / max).clamp(0.0, 1.0)
        }
    }

    /// Resolve wl-paste executable path, checking PATH, Nix profiles, and Nix store.
    pub fn find_wl_paste() -> Option<&'static str> {
        static FOUND: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
        *FOUND.get_or_init(|| {
            let candidates = [
                "wl-paste",
                "/run/current-system/sw/bin/wl-paste",
                "/etc/profiles/per-user/CheeseGuru/bin/wl-paste",
                "/nix/var/nix/profiles/default/bin/wl-paste",
                "/nix/store/5biz40pc9n1r463bqnky55vjp42bxg76-wl-clipboard-2.3.0/bin/wl-paste",
            ];
            for bin in candidates {
                if let Ok(out) = Command::new(bin).arg("--version").output() {
                    if out.status.success() {
                        tracing::info!("Found working wl-paste at: {}", bin);
                        return Some(bin);
                    }
                }
            }
            None
        })
    }

    /// Fetch current clipboard text using wl-paste.
    pub fn get_clipboard_text() -> Option<String> {
        let bin = Self::find_wl_paste()?;
        if let Ok(output) = Command::new(bin).arg("--no-newline").output() {
            if output.status.success() {
                if let Ok(text) = String::from_utf8(output.stdout) {
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
        }
        None
    }

    /// Resolve wl-copy executable path, checking PATH, Nix profiles, and Nix store.
    pub fn find_wl_copy() -> Option<&'static str> {
        static FOUND: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
        *FOUND.get_or_init(|| {
            let candidates = [
                "wl-copy",
                "/run/current-system/sw/bin/wl-copy",
                "/etc/profiles/per-user/CheeseGuru/bin/wl-copy",
                "/nix/var/nix/profiles/default/bin/wl-copy",
                "/nix/store/5biz40pc9n1r463bqnky55vjp42bxg76-wl-clipboard-2.3.0/bin/wl-copy",
            ];
            for bin in candidates {
                if let Ok(out) = Command::new(bin).arg("--version").output() {
                    if out.status.success() {
                        tracing::info!("Found working wl-copy at: {}", bin);
                        return Some(bin);
                    }
                }
            }
            None
        })
    }

    /// Copy bytes into a Wayland selection via `wl-copy`, piping over stdin
    /// so arbitrarily long texts never hit command-line length limits.
    /// No trim flags: wl-copy preserves exact bytes by default, and it
    /// daemonizes itself to serve the selection.
    fn wl_copy_to_selection(text: &str, primary: bool) -> bool {
        let bin = match Self::find_wl_copy() {
            Some(b) => b,
            None => {
                tracing::warn!("wl-copy not found; cannot seed clipboard");
                return false;
            }
        };
        let mut cmd = Command::new(bin);
        if primary {
            cmd.arg("--primary");
        }
        let mut child = match cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("failed to spawn wl-copy: {e:?}");
                return false;
            }
        };
        if let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(text.as_bytes()).is_err()
        {
            tracing::warn!("failed to pipe text to wl-copy");
            return false;
        }
        // Dropping stdin delivers EOF; wait for wl-copy to take over serving.
        match child.wait() {
            Ok(status) if status.success() => true,
            _ => {
                tracing::warn!("wl-copy exited unsuccessfully");
                false
            }
        }
    }

    /// Seed BOTH Wayland selections so every app family can paste: the
    /// regular CLIPBOARD (GTK/Qt/Ctrl+V) and PRIMARY (terminals' Shift+Insert).
    /// Returns true if at least the CLIPBOARD seed succeeded.
    pub fn seed_clipboard_both(text: &str) -> bool {
        let clip = Self::wl_copy_to_selection(text, false);
        if !Self::wl_copy_to_selection(text, true) {
            tracing::warn!("wl-copy --primary failed; terminal Shift+Insert may miss");
        }
        clip
    }

    /// Query Hyprland for the focused window class via `.socket.sock`.
    /// Uses the `j/activewindow` request (the `j/` prefix selects JSON
    /// output; plain `activewindow` returns human-readable text). Falls back
    /// to parsing `\tclass: ...` lines from the plain format. Returns the
    /// lowercase class, or None when Hyprland is unreachable or nothing is
    /// focused. Bounded by short socket timeouts so a hung compositor can't
    /// stall input.
    pub fn active_window_class() -> Option<String> {
        let path = PathBuf::from(std::env::var("XDG_RUNTIME_DIR").ok()?)
            .join("hypr")
            .join(std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?)
            .join(".socket.sock");
        if !path.exists() {
            return None;
        }
        let mut stream = std::os::unix::net::UnixStream::connect(&path).ok()?;
        stream
            .set_write_timeout(Some(Duration::from_millis(200)))
            .ok()?;
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .ok()?;
        stream.write_all(b"j/activewindow").ok()?;
        // Read until a complete JSON object arrives, the peer closes, or the
        // deadline passes; then parse whatever we got.
        let deadline = Instant::now() + Duration::from_millis(300);
        let mut buf = Vec::with_capacity(4096);
        let mut tmp = [0u8; 4096];
        loop {
            match stream.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.len() > 65536 {
                        break;
                    }
                    if String::from_utf8_lossy(&buf).trim_end().ends_with('}') {
                        break;
                    }
                }
                Err(_) => break,
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        let reply = String::from_utf8_lossy(&buf);
        // Primary: JSON `{"class": "...", ...}`.
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(reply.trim())
            && let Some(class) = v.get("class").and_then(|c| c.as_str())
        {
            return Some(class.to_lowercase());
        }
        // Fallback: plain-text lines like `\tclass: com.mitchellh.ghostty`.
        for line in reply.lines() {
            if let Some(rest) = line.trim_start().strip_prefix("class:")
                && let Some(class) = rest.trim().split_whitespace().next()
                && !class.is_empty()
            {
                return Some(class.to_lowercase());
            }
        }
        None
    }

    /// True for terminal emulators, where the clipboard paste shortcut is
    /// Ctrl+Shift+V (ghostty/foot/kitty/alacritty/...). Plain Ctrl+V is
    /// unreliable there (often swallowed or sent as raw ASCII), and
    /// Shift+Insert hits PRIMARY instead of CLIPBOARD in several of them,
    /// so terminals uniformly get Ctrl+Shift+V.
    fn is_terminal_class(class: &str) -> bool {
        const TERMS: &[&str] = &[
            "ghostty",
            "foot",
            "footclient",
            "kitty",
            "alacritty",
            "wezterm",
            "gnome-terminal",
            "gnome-terminal-server",
            "konsole",
            "xfce4-terminal",
            "tilix",
            "terminator",
            "xterm",
            "uxterm",
            "rxvt",
            "urxvt",
            "st",
            "sakura",
            "hyper",
            "tabby",
            "warp",
            "rio",
        ];
        TERMS.iter().any(|t| class.contains(t)) || class.contains("terminal")
    }

    /// Emit Shift+Insert: pastes the PRIMARY selection in ghostty/foot/
    /// alacritty/xterm/gnome-terminal and the regular clipboard in standard
    /// GTK/Qt text fields. Reuses the modifier-aware sender, so latched/held
    /// mods compose and one-shot mods are consumed like a normal key tap.
    pub fn send_shift_insert(&mut self) {
        self.send_key_with_modifiers(KEY_INSERT, true);
    }

    /// Emit Ctrl+V: pastes the regular clipboard in GTK/Qt/Chromium/Firefox.
    /// Engages Ctrl one-shot for this emission only; held mods survive.
    pub fn send_ctrl_v(&mut self) {
        if (self.pending_mods | self.held_mods) & Self::MOD_CTRL == 0 {
            self.pending_mods |= Self::MOD_CTRL;
        }
        self.send_key_with_modifiers(KEY_V, false);
    }

    /// Emit Ctrl+Shift+V: clipboard paste in terminals (kitty leaves
    /// Shift+Insert unbound; ghostty/foot use it for PRIMARY instead).
    /// Same one-shot composition as Ctrl+V.
    pub fn send_ctrl_shift_v(&mut self) {
        if (self.pending_mods | self.held_mods) & Self::MOD_CTRL == 0 {
            self.pending_mods |= Self::MOD_CTRL;
        }
        self.send_key_with_modifiers(KEY_V, true);
    }

    /// Instant, atomic clipboard paste — always a real clipboard paste, never
    /// an IME `commit_string` (which apps like Ghostty replay line by line).
    ///
    /// Seeds BOTH selections via wl-copy (CLIPBOARD + PRIMARY), then fires
    /// the paste shortcut matching the focused app: Ctrl+Shift+V for
    /// terminals (Shift+Insert is unbound in kitty and hits PRIMARY, not
    /// CLIPBOARD, in ghostty/foot/gnome-terminal), Ctrl+V for GUI apps and
    /// browsers (CLIPBOARD). Falls back to char-by-char `send_text` only
    /// when `wl-copy` is missing or fails.
    pub fn paste_text_instant(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if Self::seed_clipboard_both(text) {
            let class = Self::active_window_class().unwrap_or_default();
            if Self::is_terminal_class(&class) {
                self.send_ctrl_shift_v();
            } else {
                self.send_ctrl_v();
            }
            tracing::info!(
                "paste_text_instant via clipboard ({} chars, class='{}')",
                text.chars().count(),
                class
            );
            return;
        }
        tracing::info!("paste_text_instant falling back to send_text");
        self.send_text(text);
    }

    /// Path to persistent clipboard history JSON file.
    pub fn clipboard_cache_path() -> Option<PathBuf> {
        let base = if let Ok(cache) = std::env::var("XDG_CACHE_HOME") {
            PathBuf::from(cache).join("hyprosk")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".cache/hyprosk")
        } else {
            return None;
        };
        Some(base.join("clipboard_history.json"))
    }

    /// Load persistent clipboard history from disk.
    pub fn load_clipboard_history() -> VecDeque<String> {
        if let Some(path) = Self::clipboard_cache_path() {
            if path.exists() {
                if let Ok(data) = std::fs::read_to_string(&path) {
                    if let Ok(items) = serde_json::from_str::<Vec<String>>(&data) {
                        return items.into();
                    }
                }
            }
        }
        VecDeque::new()
    }

    /// Save persistent clipboard history to disk.
    pub fn save_clipboard_history(&self) {
        if let Some(path) = Self::clipboard_cache_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let items: Vec<String> = self.clipboard_history.iter().cloned().collect();
            if let Ok(json) = serde_json::to_string_pretty(&items) {
                let _ = std::fs::write(&path, json);
            }
        }
    }

    /// Record the current clipboard content into the (capped) history.
    pub fn capture_clipboard(&mut self) {
        if let Some(text) = Self::get_clipboard_text() {
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
        self.save_clipboard_history();
    }

    pub fn switch_layer(&mut self, layer: LayerId, qh: &QueueHandle<Self>) {
        self.release_all_held_mods();
        self.pressed_keys.clear();
        self.pressed_key = None;
        self.press_instant = None;
        self.hold_preview = None;
        self.current_layer = layer;
        self.sync_layout_and_redraw(qh);
    }

    #[inline]
    pub fn update_key_rects(&mut self) {
        if self.clipboard_mode {
            let max = RenderEngine::clipboard_scroll_metrics(
                &self.layout,
                self.width,
                self.height,
                &self.theme,
            )
            .map(|(_, _, _, m)| m)
            .unwrap_or(0.0);
            self.clipboard_scroll_y = self.clipboard_scroll_y.clamp(0.0, max.max(0.0));
            self.key_rects = RenderEngine::calculate_key_rects_with_scroll(
                &self.layout,
                self.width,
                self.height,
                &self.theme,
                self.clipboard_scroll_y,
            );
        } else {
            self.key_rects = RenderEngine::calculate_key_rects(
                &self.layout,
                self.width,
                self.height,
                &self.theme,
            );
        }
    }

    fn layout_suggestions(&self) -> Vec<String> {
        if let Some(preview) = &self.swipe_preview {
            vec![String::new(), preview.clone(), String::new()]
        } else {
            self.suggest_engine.candidates.clone()
        }
    }

    pub fn sync_layout_and_redraw(&mut self, qh: &QueueHandle<Self>) {
        let suggestions = self.layout_suggestions();
        if self.clipboard_mode {
            let history: Vec<String> = self.clipboard_history.iter().cloned().collect();
            let padding_x = self.custom_layout.as_ref().and_then(|custom| custom.padding_x);
            let mut lay = KeyboardLayout::clipboard(&history, &suggestions, self.width, padding_x);
            if let Some(ref custom) = self.custom_layout {
                lay.colors = custom.colors.as_ref().map(|c| c.resolved(&custom.m3_map));
                lay.key_spacing = custom.key_spacing;
                lay.row_spacing = custom.row_spacing;
                lay.padding_x = custom.padding_x;
                lay.padding_y = custom.padding_y;
                lay.borders = custom.borders;
                lay.blur = custom.blur;
                lay.opacity = custom.opacity;
                lay.key_opacity = custom.key_opacity;
            }
            self.layout = lay;
        } else if let Some(ref custom) = self.custom_layout {
            self.layout =
                custom.to_keyboard_layout(self.current_layer, &suggestions, self.caps_lock);
        } else {
            self.layout = KeyboardLayout::get_layout_for_mode(
                self.current_layer,
                &suggestions,
                self.caps_lock,
                self.layout_mode,
            );
        }
        self.update_key_rects();
        self.redraw(qh);
    }

    pub fn adapt_layout_for_content_purpose(&mut self, purpose: u32, qh: &QueueHandle<Self>) {
        if self.im_state.last_content_purpose == Some(purpose) {
            return;
        }
        self.im_state.last_content_purpose = Some(purpose);

        // Wayland zwp_text_input_v3 content_purpose values:
        // 0: normal, 1: alpha, 2: digits, 3: number, 4: phone, 5: url, 6: email,
        // 7: name, 8: password, 9: pin, 10: date, 11: time, 12: datetime, 13: terminal
        // Only numeric input modes should auto-open on the Symbols layer.
        let is_numeric = matches!(purpose, 2 | 3 | 4 | 9);
        if is_numeric {
            if self.current_layer != LayerId::Symbols && self.current_layer != LayerId::Symbols2 {
                self.switch_layer(LayerId::Symbols, qh);
            }
        } else if (self.current_layer == LayerId::Symbols
            || self.current_layer == LayerId::Symbols2)
            && matches!(purpose, 0 | 1 | 5 | 6 | 7 | 8 | 13)
        {
            // Textual fields (normal, alpha, url, email, etc.) default back to Lower
            self.switch_layer(LayerId::Lower, qh);
        }
    }

    pub fn redraw(&mut self, _qh: &QueueHandle<Self>) {
        if !self.is_visible || !self.is_configured {
            return;
        }

        let redraw_started = Instant::now();
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
            if self.pool.len() < needed
                && let Err(e) = self.pool.resize(needed)
            {
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
            self.base_frame.clear();
        }

        let visual_pressed_keys = self.touch_visual_pressed_keys();
        let latched_keys = self.latched_positions();
        // Draw into the first buffer whose slot is free (i.e. released by the
        // compositor). If all are still in flight, skip this frame — the
        // previously committed buffer remains displayed.
        for buffer in &self.canvas_buffers {
            let Some(canvas) = buffer.canvas(&mut self.pool) else {
                continue;
            };

            let mut slint_damage: Option<Vec<(i32, i32, i32, i32)>> = None;
            let frame_len = height as usize * stride as usize;
            let cached_base_valid = self.base_frame.len() == frame_len;
            self.base_frame.resize(frame_len, 0);
            let mut slint_rendered = false;
            let mut restored_cached_base = false;
            if self.trail_only_redraw && cached_base_valid {
                canvas.copy_from_slice(&self.base_frame);
                restored_cached_base = true;
                slint_damage = Some(Vec::new());
            }
            // Paint swap: render the Wireframe scene through Slint when available.
            if slint_damage.is_none() && self.slint_scene.is_none() {
                match SlintScene::new(width, height) {
                    Ok(scene) => self.slint_scene = Some(scene),
                    Err(e) => {
                        tracing::warn!("Slint renderer unavailable, using legacy renderer: {e:?}");
                    }
                }
            }
            if slint_damage.is_none()
                && let Some(scene) = self.slint_scene.as_mut()
            {
                if let Some(damage) = scene.render(
                    &self.layout,
                    &self.theme,
                    width,
                    height,
                    &visual_pressed_keys,
                    &latched_keys,
                    self.swipe_offset,
                    self.hold_preview.clone(),
                    self.clipboard_scroll_y,
                    &mut self.base_frame,
                ) {
                    slint_damage = Some(damage);
                    slint_rendered = true;
                    tracing::trace!("Painted frame via Slint scene ({}x{})", width, height);
                } else {
                    tracing::warn!("Slint frame render failed, falling back to legacy renderer");
                }
            }

            if slint_damage.is_none() {
                RenderEngine::render(
                    canvas,
                    width,
                    height,
                    &self.layout,
                    &self.theme,
                    &visual_pressed_keys,
                    &latched_keys,
                    self.swipe_offset,
                );
            }
            if slint_rendered {
                canvas.copy_from_slice(&self.base_frame);
            }
            if !restored_cached_base && !slint_rendered {
                self.base_frame.clear();
                self.base_frame.extend_from_slice(canvas);
            }
            self.trail_only_redraw = false;

            let now = Instant::now();
            let trail: Vec<(f32, f32, Instant)> = self
                .swipe_traces
                .values()
                .find(|trace| trace.qualifies())
                .map(|trace| &trace.points)
                .or(self.fading_swipe_trail.as_ref())
                .map(|points| {
                    points
                        .iter()
                        .filter(|(_, _, at)| {
                            now.saturating_duration_since(*at) < Duration::from_millis(420)
                        })
                        .rev()
                        .take(96)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .map(|(x, y, at)| (*x as f32, *y as f32, *at))
                        .collect()
                })
                .unwrap_or_default();
            if trail.len() > 1 {
                let color = self
                    .layout
                    .colors
                    .as_ref()
                    .and_then(|c| c.swipe_trail_color.as_deref().or(c.accent_color.as_deref()))
                    .map(Color::parse_hex)
                    .unwrap_or(self.theme.accent_color);
                let trail_width = self
                    .layout
                    .swipe
                    .as_ref()
                    .and_then(|swipe| swipe.trail.as_ref())
                    .and_then(|trail| trail.width)
                    .unwrap_or(RenderEngine::DEFAULT_SWIPE_TRAIL_WIDTH);
                RenderEngine::draw_swipe_trail(
                    canvas,
                    width,
                    height,
                    &trail,
                    now,
                    color,
                    trail_width,
                    &mut self.trail_alpha_mask,
                );
                slint_damage = Some(Vec::new());
            }

            if let Some(ref surface) = self.layer_surface
                && buffer.attach_to(surface.wl_surface()).is_ok()
            {
                if let Some(ref rects) = slint_damage
                    && !rects.is_empty()
                {
                    for &(dx, dy, dw, dh) in rects {
                        surface.wl_surface().damage_buffer(dx, dy, dw, dh);
                    }
                } else {
                    surface
                        .wl_surface()
                        .damage_buffer(0, 0, width as i32, height as i32);
                }
                surface.commit();
            }
            break;
        }
        let elapsed = redraw_started.elapsed();
        if elapsed >= Duration::from_millis(8) {
            tracing::debug!(
                elapsed_ms = elapsed.as_secs_f64() * 1000.0,
                "slow keyboard redraw"
            );
        }
    }

    pub fn handle_key_press(
        &mut self,
        r_idx: usize,
        k_idx: usize,
        qh: &QueueHandle<Self>,
        touch_id: i32,
    ) {
        let action = {
            let row = match self.layout.rows.get(r_idx) {
                Some(r) => r,
                None => return,
            };
            let key = match row.keys.get(k_idx) {
                Some(k) => k,
                None => return,
            };
            key.action.clone()
        };

        tracing::debug!("Key pressed: {:?}", action);
        self.pressed_key = Some((r_idx, k_idx));
        if !self.pressed_keys.contains(&(r_idx, k_idx)) {
            self.pressed_keys.push((r_idx, k_idx));
        }
        if !matches!(action, KeyAction::Shift) {
            self.last_shift_press = None;
        }
        // Any non-modifier press while modifiers are held counts as a chord:
        // held bits survive the emission instead of converting to a tap-latch.
        let is_modifier = matches!(
            action,
            KeyAction::Shift | KeyAction::Ctrl | KeyAction::Alt | KeyAction::Win
        );
        if !is_modifier && self.held_mods != 0 {
            self.held_mod_chorded |= self.held_mods;
        }

        match action {
            KeyAction::Text(text) => {
                self.last_arrow_at = None;
                let has_command_mods = ((self.pending_mods | self.held_mods)
                    & (Self::MOD_CTRL | Self::MOD_ALT | Self::MOD_SUPER))
                    != 0;
                if !has_command_mods {
                    for ch in text.chars() {
                        self.suggest_engine.push_char(ch);
                    }
                }
                self.send_text(&text);
                // One-shot Shift reverts after a single letter, but a
                // physically-held Shift keeps Upper across many presses.
                if !self.caps_lock
                    && self.current_layer == LayerId::Upper
                    && (self.held_mods & Self::MOD_SHIFT) == 0
                {
                    self.current_layer = LayerId::Lower;
                    self.pending_mods &= !Self::MOD_SHIFT;
                    self.sync_mods();
                }
            }
            KeyAction::Suggestion(idx) => {
                if let Some(chosen_word) = self.suggest_engine.candidates.get(idx).cloned() {
                    let cur = self.suggest_engine.current_word.clone();
                    let is_next_word = self.suggest_engine.is_next_word_mode;
                                        tracing::debug!(candidate_index = idx, is_next_word, "suggestion selected");
                    if is_next_word {
                        self.suggest_engine.on_word_selected(&chosen_word);
                        self.send_text(&format!("{} ", chosen_word));
                    } else {
                        let (delete_count, replace_range) = calculate_replacement_target(
                            &cur,
                            self.im_state.surrounding_text.as_deref(),
                            self.im_state.cursor_pos,
                            self.im_state.anchor_pos,
                        );
                        let app_word = self
                            .im_state
                            .surrounding_text
                            .as_deref()
                            .and_then(|text| {
                                replace_range.map(|(start, end)| text[start..end].to_string())
                            })
                            .unwrap_or_else(|| cur.clone());
                        let has_selection = self.im_state.cursor_pos != self.im_state.anchor_pos;

                        // Replace the whole token when the caret sits inside it.
                        if !has_selection
                            && candidate_suffix_for_prefix(&cur, &chosen_word).is_some()
                            && let Some((start, end, backspaces, forward_deletes)) = self
                                .im_state
                                .surrounding_text
                                .as_deref()
                                .and_then(|text| {
                                    token_range_around_cursor(
                                        text,
                                        self.im_state.cursor_pos,
                                        self.im_state.anchor_pos,
                                    )
                                    .filter(|(start, _, cursor, _, _)| {
                                        text[*start..*cursor].eq_ignore_ascii_case(&cur)
                                    })
                                    .map(|(start, end, _, backspaces, forward_deletes)| {
                                        (start, end, backspaces, forward_deletes)
                                    })
                                })
                        {
                            let needs_space = self.im_state.surrounding_text.as_deref()
                                .and_then(|text| text.get(end..))
                                .is_none_or(str::is_empty);
                            let replacement = format!("{}{}", chosen_word, if needs_space { " " } else { "" });
                            tracing::info!(
                                delete_before = backspaces,
                                delete_after = forward_deletes,
                                replacement_bytes = replacement.len(),
                                "replacing token at the caret"
                            );
                            for _ in 0..forward_deletes {
                                self.send_keycode(KEY_DELETE);
                            }
                            for _ in 0..backspaces {
                                self.send_backspace();
                            }
                            for ch in replacement.chars() {
                                if let Some((keycode, shift)) = char_to_keycode_and_shift(ch) {
                                    self.send_key_with_modifiers(keycode, shift);
                                } else {
                                    self.send_text(&ch.to_string());
                                }
                            }
                            if let Some(ref mut text) = self.im_state.surrounding_text
                                && start <= end
                                && end <= text.len()
                                && text.is_char_boundary(start)
                                && text.is_char_boundary(end)
                            {
                                text.replace_range(start..end, &replacement);
                                let new_cursor = (start + replacement.len()) as u32;
                                self.im_state.cursor_pos = new_cursor;
                                self.im_state.anchor_pos = new_cursor;
                            }
                            self.suggest_engine.on_word_selected(&chosen_word);
                            self.sync_layout_and_redraw(qh);
                            return;
                        }

                        // Helium may hide its inline selection from Wayland.
                        // Preserve the tracked prefix and commit only the suffix.
                        if let Some(completion_suffix) =
                            candidate_suffix_for_prefix(&cur, &chosen_word)
                        {
                            let completion_range = if has_selection {
                                host_completion_suffix(
                                    self.im_state.surrounding_text.as_deref(),
                                    self.im_state.cursor_pos,
                                    self.im_state.anchor_pos,
                                    &chosen_word,
                                )
                                .map(|(_, range)| range)
                            } else {
                                None
                            };
                            let replacement = format!("{} ", completion_suffix);
                                                        tracing::info!(replacement_bytes = replacement.len(), "suggestion prefix completed");
                            self.send_text(&replacement);
                            if let Some((start, end)) = completion_range
                                && let Some(ref mut text) = self.im_state.surrounding_text
                                && start <= end
                                && end <= text.len()
                                && text.is_char_boundary(start)
                                && text.is_char_boundary(end)
                            {
                                text.replace_range(start..end, &replacement);
                                let new_cursor = (start + replacement.len()) as u32;
                                self.im_state.cursor_pos = new_cursor;
                                self.im_state.anchor_pos = new_cursor;
                            } else if !has_selection
                                && let Some(ref mut text) = self.im_state.surrounding_text
                            {
                                let pos = text.floor_char_boundary(
                                    (self.im_state.cursor_pos as usize).min(text.len()),
                                );
                                if text[..pos].ends_with(&cur) {
                                    text.insert_str(pos, &replacement);
                                    let new_cursor = (pos + replacement.len()) as u32;
                                    self.im_state.cursor_pos = new_cursor;
                                    self.im_state.anchor_pos = new_cursor;
                                }
                            }
                            self.suggest_engine.on_word_selected(&chosen_word);
                            self.sync_layout_and_redraw(qh);
                            return;
                        }

                        // Replace only the selected browser-completion suffix.
                        if has_selection
                            && let Some((completion_suffix, (start, end))) = host_completion_suffix(
                                self.im_state.surrounding_text.as_deref(),
                                self.im_state.cursor_pos,
                                self.im_state.anchor_pos,
                                &chosen_word,
                            )
                        {
                            let replacement = format!("{} ", completion_suffix);
                                                        tracing::info!(replacement_bytes = replacement.len(), selection_start = start, selection_end = end, "suggestion host completion");
                            for ch in replacement.chars() {
                                if let Some((keycode, shift)) = char_to_keycode_and_shift(ch) {
                                    self.send_key_with_modifiers(keycode, shift);
                                } else {
                                    self.send_text(&ch.to_string());
                                }
                            }
                            if let Some(ref mut text) = self.im_state.surrounding_text
                                && start <= end
                                && end <= text.len()
                                && text.is_char_boundary(start)
                                && text.is_char_boundary(end)
                            {
                                text.replace_range(start..end, &replacement);
                                let new_cursor = (start + replacement.len()) as u32;
                                self.im_state.cursor_pos = new_cursor;
                                self.im_state.anchor_pos = new_cursor;
                            }
                            self.suggest_engine.on_word_selected(&chosen_word);
                            self.sync_layout_and_redraw(qh);
                            return;
                        }

                        // A selected host completion must still be replaced even
                        // when it happens to equal our chosen word: the typed
                        // prefix remains outside the host's selected suffix.
                        if !has_selection && chosen_word == app_word {
                            tracing::info!("suggest same -> space");
                            self.suggest_engine.on_word_selected(&chosen_word);
                            self.send_text(" ");
                        } else {
                            // Unified virtual-keyboard replacement: Chromium/Ozone Wayland drops
                            // zwp_input_method_v2::delete_surrounding_text, and mixing VK
                            // backspaces with im.commit_string races across protocols
                            // (leftover chars / 'bbro'). Keeping deletes + inserts on the
                            // single VK event stream guarantees FIFO ordering.
                            let replacement = format!("{} ", chosen_word);
                                                        tracing::info!(delete_count, replacement_bytes = replacement.len(), "suggestion virtual-keyboard replacement");
                            for _ in 0..delete_count {
                                self.send_backspace();
                            }
                            for ch in replacement.chars() {
                                if let Some((keycode, shift)) = char_to_keycode_and_shift(ch) {
                                    self.send_key_with_modifiers(keycode, shift);
                                } else {
                                    self.send_text(&ch.to_string());
                                }
                            }
                            if let Some(ref mut text) = self.im_state.surrounding_text {
                                if let Some((start, end)) = replace_range {
                                    if start <= end
                                        && end <= text.len()
                                        && text.is_char_boundary(start)
                                        && text.is_char_boundary(end)
                                    {
                                        text.replace_range(start..end, &replacement);
                                        let new_cursor = (start + replacement.len()) as u32;
                                        self.im_state.cursor_pos = new_cursor;
                                        self.im_state.anchor_pos = new_cursor;
                                    }
                                } else {
                                    let pos = text.floor_char_boundary(
                                        (self.im_state.cursor_pos as usize).min(text.len()),
                                    );
                                    let before_pos = pos.saturating_sub(cur.len());
                                    if text.is_char_boundary(before_pos)
                                        && text.is_char_boundary(pos)
                                        && text[..pos].ends_with(&cur)
                                    {
                                        text.replace_range(before_pos..pos, &replacement);
                                        let new_cursor = (before_pos + replacement.len()) as u32;
                                        self.im_state.cursor_pos = new_cursor;
                                        self.im_state.anchor_pos = new_cursor;
                                    }
                                }
                            }
                            self.suggest_engine.on_word_selected(&chosen_word);
                        }
                    }
                }
            }
            KeyAction::Backspace => {
                self.suggest_engine.pop_char();
                self.send_backspace();
                self.pop_surrounding_char();
                self.repeat_key = Some(KeyAction::Backspace);
                self.repeat_start = Some(Instant::now());
                self.last_repeat_at = Some(Instant::now());
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
                self.suggest_engine.clear();
                self.send_escape();
            }
            KeyAction::Shift => {
                // Hold-tracked: layer raises now for chord feedback; the
                // tap toggle / double-tap CapsLock resolves on release.
                self.shift_down(touch_id, qh);
                return;
            }
            KeyAction::SwitchLayer(layer) => {
                // A layer switch rebuilds all geometry: drop held modifiers
                // first so no physical key or latch can get stuck.
                self.release_all_held_mods();
                self.pressed_keys.clear();
                self.pressed_key = None;
                self.press_instant = None;
                self.hold_preview = None;
                self.caps_lock = false;
                self.current_layer = layer;
                // Layer keys inside the clipboard view (e.g. the ABC bottom-row
                // key) return to normal letter typing.
                if self.clipboard_mode {
                    self.clipboard_mode = false;
                }
            }
            KeyAction::Hide => {
                self.manually_shown = false;
                self.hide_keyboard(qh);
            }
            KeyAction::ArrowLeft => {
                self.send_arrow_left(qh);
                self.repeat_key = Some(KeyAction::ArrowLeft);
                self.repeat_start = Some(Instant::now());
                self.last_repeat_at = Some(Instant::now());
            }
            KeyAction::ArrowRight => {
                self.send_arrow_right(qh);
                self.repeat_key = Some(KeyAction::ArrowRight);
                self.repeat_start = Some(Instant::now());
                self.last_repeat_at = Some(Instant::now());
            }
            KeyAction::ArrowUp => {
                self.send_arrow_up();
                self.last_arrow_at = Some(Instant::now());
                self.suggest_engine.clear();
                self.im_state.surrounding_text = None;
                self.im_state.cursor_pos = 0;
                self.im_state.anchor_pos = 0;
                self.sync_layout_and_redraw(qh);
                self.repeat_key = Some(KeyAction::ArrowUp);
                self.repeat_start = Some(Instant::now());
                self.last_repeat_at = Some(Instant::now());
            }
            KeyAction::ArrowDown => {
                self.send_arrow_down();
                self.last_arrow_at = Some(Instant::now());
                self.suggest_engine.clear();
                self.im_state.surrounding_text = None;
                self.im_state.cursor_pos = 0;
                self.im_state.anchor_pos = 0;
                self.sync_layout_and_redraw(qh);
                self.repeat_key = Some(KeyAction::ArrowDown);
                self.repeat_start = Some(Instant::now());
                self.last_repeat_at = Some(Instant::now());
            }
            KeyAction::Copy => {
                self.capture_clipboard();
            }
            KeyAction::Paste => {
                tracing::info!("Paste requested");
                if let Some(text) = Self::get_clipboard_text() {
                    self.paste_text_instant(&text);
                    self.push_clipboard(text);
                }
            }
            KeyAction::Clipboard => {
                self.toggle_clipboard(qh);
            }
            KeyAction::ClipboardItem(idx) => {
                if let Some(text) = self.clipboard_history.get(idx).cloned() {
                    self.paste_text_instant(&text);
                }
                self.clipboard_mode = false;
                self.pressed_keys.clear();
                self.pressed_key = None;
                self.press_instant = None;
                self.hold_preview = None;
                self.current_layer = LayerId::Lower;
            }
            KeyAction::Ctrl => {
                // Hold-tracked: tap converts to a one-shot latch on release.
                self.mod_down(Self::MOD_CTRL, touch_id, qh);
                return;
            }
            KeyAction::Alt => {
                self.mod_down(Self::MOD_ALT, touch_id, qh);
                return;
            }
            KeyAction::Win => {
                self.mod_down(Self::MOD_SUPER, touch_id, qh);
                return;
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
        let is_space_release = if let Some(row) = self.layout.rows.get(r_idx)
            && let Some(key) = row.keys.get(k_idx)
        {
            matches!(key.action, KeyAction::Space) && !self.is_space_swiping
        } else {
            false
        };

        if is_space_release {
            self.suggest_engine.on_space();
            self.send_space();
        }

        if let Some(row) = self.layout.rows.get(r_idx)
            && let Some(key) = row.keys.get(k_idx)
            && matches!(key.action, KeyAction::ArrowUp | KeyAction::ArrowDown)
        {
            self.last_arrow_at = Some(Instant::now());
        }

        self.repeat_key = None;
        self.repeat_start = None;
        self.last_repeat_at = None;
        self.pressed_keys.retain(|&p| p != (r_idx, k_idx));
        self.pressed_key = self.pressed_keys.last().copied();
        if self.pressed_keys.is_empty() {
            self.press_instant = None;
            self.hold_preview = None;
            self.space_touch_start = None;
            self.is_space_swiping = false;
            self.swipe_offset = None;
        }
        self.sync_layout_and_redraw(qh);
    }

    pub fn sync_suggest_with_atspi(
        &mut self,
        snapshot: crate::atspi::Snapshot,
        qh: &QueueHandle<Self>,
    ) {
        if !self.im_state.is_active.load(Ordering::Relaxed)
            || matches!(self.im_state.content_purpose, 8 | 9)
            || self.im_state.content_hint & (0x40 | 0x80) != 0
            || snapshot.text.len() > 16 * 1024
        {
            return;
        }
        if self
            .im_state
            .last_wayland_snapshot_at
            .is_some_and(|last| last.elapsed() < Duration::from_millis(500))
        {
            return;
        }
        let cursor = snapshot.cursor_bytes;
        let Ok(cursor_u32) = u32::try_from(cursor) else {
            return;
        };
        if cursor > snapshot.text.len() || !snapshot.text.is_char_boundary(cursor) {
            return;
        }
        if self.im_state.surrounding_text.as_deref() == Some(&snapshot.text)
            && self.im_state.cursor_pos == cursor_u32
            && self.im_state.anchor_pos == cursor_u32
        {
            return;
        }

        self.im_state.surrounding_text = Some(snapshot.text.clone());
        self.im_state.cursor_pos = cursor_u32;
        self.im_state.anchor_pos = cursor_u32;
        self.sync_suggest_with_surrounding(&snapshot.text, cursor_u32, true, qh);
    }

    pub fn sync_suggest_with_surrounding(
        &mut self,
        text: &str,
        cursor: u32,
        authoritative: bool,
        qh: &QueueHandle<Self>,
    ) {
        tracing::info!(
            surrounding_bytes = text.len(),
            cursor,
            authoritative,
            "syncing suggestions from host surrounding text"
        );
        if authoritative {
            self.suggest_engine
                .sync_with_external_text(text, cursor as usize);
        } else {
            self.suggest_engine.sync_with_text(text, cursor as usize);
        }
        tracing::info!(
            current_word_bytes = self.suggest_engine.current_word.len(),
            candidate_count = self.suggest_engine.candidates.len(),
            next_word_mode = self.suggest_engine.is_next_word_mode,
            "suggestion context updated"
        );
        self.sync_layout_and_redraw(qh);
    }

    pub fn update_key_repeat(&mut self, qh: &QueueHandle<Self>) {
        if let Some(action) = self.repeat_key.clone()
            && let Some(start) = self.repeat_start
        {
            let delay = Duration::from_millis(self.config.behavior.repeat_delay_ms.max(200));
            let rate = Duration::from_millis(self.config.behavior.repeat_rate_ms.max(30));
            if start.elapsed() >= delay {
                let now = Instant::now();
                let should_fire = match self.last_repeat_at {
                    Some(last) => now.duration_since(last) >= rate,
                    None => true,
                };
                if should_fire {
                    match action {
                        KeyAction::Backspace => {
                            self.suggest_engine.pop_char();
                            self.send_backspace();
                            self.pop_surrounding_char();
                            self.sync_layout_and_redraw(qh);
                        }
                        KeyAction::ArrowLeft => {
                            self.send_arrow_left(qh);
                        }
                        KeyAction::ArrowRight => {
                            self.send_arrow_right(qh);
                        }
                        KeyAction::ArrowUp => {
                            self.send_arrow_up();
                            self.last_arrow_at = Some(Instant::now());
                            self.suggest_engine.clear();
                            self.im_state.surrounding_text = None;
                            self.im_state.cursor_pos = 0;
                            self.im_state.anchor_pos = 0;
                            self.sync_layout_and_redraw(qh);
                        }
                        KeyAction::ArrowDown => {
                            self.send_arrow_down();
                            self.last_arrow_at = Some(Instant::now());
                            self.suggest_engine.clear();
                            self.im_state.surrounding_text = None;
                            self.im_state.cursor_pos = 0;
                            self.im_state.anchor_pos = 0;
                            self.sync_layout_and_redraw(qh);
                        }
                        _ => {}
                    }
                    self.last_repeat_at = Some(now);
                }
            }
        }
    }

    #[cfg(feature = "futo-swipe")]
    fn invalidate_swipe_preview(&mut self) {
        self.pending_swipe_preview = None;
        self.swipe_preview_sequence = self.swipe_preview_sequence.wrapping_add(1);
    }

    #[cfg(feature = "futo-swipe")]
    fn queue_swipe_preview(&mut self, touch_id: i32, trace: &SwipeTrace) {
        let Some(worker) = self.swipe_preview_worker.as_ref() else {
            return;
        };
        let Some((keys, aspect, points)) = self.swipe_decode_input(trace) else {
            self.swipe_preview = None;
            return;
        };
        self.swipe_preview_sequence = self.swipe_preview_sequence.wrapping_add(1);
        let sequence = self.swipe_preview_sequence;
        let revision = self.suggest_engine.dictionary.swipe_lexicon_revision();
        let dictionary_update = if revision != self.swipe_preview_dictionary_revision {
            self.swipe_preview_dictionary_revision = revision;
            Some(self.suggest_engine.dictionary.clone())
        } else {
            None
        };
        self.pending_swipe_preview = Some((touch_id, sequence));
        worker.submit(PreviewRequest {
            touch_id,
            sequence,
            keys,
            aspect,
            points,
            dictionary_revision: revision,
            dictionary_update,
        });
    }

    #[cfg(feature = "futo-swipe")]
    pub fn update_swipe_preview(&mut self, qh: &QueueHandle<Self>) {
        let Some(worker) = self.swipe_preview_worker.as_ref() else {
            return;
        };
        let Some(result) = worker.take_result() else {
            return;
        };
        if self.pending_swipe_preview != Some((result.touch_id, result.sequence)) {
            return;
        }
        self.pending_swipe_preview = None;
        if result.dictionary_revision != self.suggest_engine.dictionary.swipe_lexicon_revision()
            || !self
                .swipe_traces
                .get(&result.touch_id)
                .is_some_and(SwipeTrace::qualifies)
        {
            return;
        }
        tracing::debug!(
            elapsed_ms = result.elapsed_ms,
            "background swipe preview decode completed"
        );
        if result.elapsed_ms >= 100.0 {
            tracing::warn!(
                elapsed_ms = result.elapsed_ms,
                "background swipe preview decode exceeded 100 ms"
            );
        }
        let best = result
            .candidates
            .first()
            .map(|word| self.swipe_case_word(word));
        if let Some(active) = self.swipe_traces.get_mut(&result.touch_id) {
            active.candidates = best.iter().cloned().collect();
        }
        self.swipe_preview = best;
        self.sync_layout_and_redraw(qh);
    }

    pub fn has_frame_activity(&self) -> bool {
        !self.active_touches.is_empty()
            || !self.swipe_traces.is_empty()
            || self.fading_swipe_trail.is_some()
            || self.repeat_key.is_some()
            || self.hold_preview.is_some()
            || !self.touch_press_feedback.is_empty()
    }

    fn same_touch_feedback_key(pressed: &KeyAction, current: &KeyAction) -> bool {
        match (pressed, current) {
            (KeyAction::Text(_), KeyAction::Text(_))
            | (KeyAction::SwitchLayer(_), KeyAction::SwitchLayer(_)) => true,
            _ => pressed == current,
        }
    }

    fn touch_visual_pressed_keys(&self) -> Vec<(usize, usize)> {
        let now = Instant::now();
        let mut pressed = self.pressed_keys.clone();
        for (&position, (action, expires_at)) in &self.touch_press_feedback {
            if *expires_at <= now || pressed.contains(&position) {
                continue;
            }
            let same_key = self
                .layout
                .rows
                .get(position.0)
                .and_then(|row| row.keys.get(position.1))
                .is_some_and(|key| Self::same_touch_feedback_key(action, &key.action));
            if same_key {
                pressed.push(position);
            }
        }
        pressed
    }

    /// Keep quick touch taps visible even if down and up arrive before a frame.
    pub fn update_touch_press_feedback(&mut self, qh: &QueueHandle<Self>) {
        if self.touch_press_feedback.is_empty() {
            return;
        }
        let now = Instant::now();
        self.touch_press_feedback
            .retain(|_, (_, expires_at)| *expires_at > now);
        self.redraw(qh);
    }

    pub fn update_swipe_trail(&mut self, qh: &QueueHandle<Self>) {
        const TRAIL_LIFETIME: Duration = Duration::from_millis(420);
        const FRAME_INTERVAL: Duration = Duration::from_millis(16);
        let now = Instant::now();

        // Touch motion events can arrive faster than the render loop. Refresh
        // the active path at display cadence, and keep animating its fade for
        // one trail lifetime after the last sample so a stationary finger does
        // not leave a frozen line on screen.
        let latest_active_sample = self
            .swipe_traces
            .values()
            .filter(|trace| trace.qualifies())
            .filter_map(|trace| trace.points.last().map(|(_, _, at)| *at))
            .max();
        if let Some(last_sample_at) = latest_active_sample {
            let trail_still_visible =
                now.saturating_duration_since(last_sample_at) < TRAIL_LIFETIME;
            if !trail_still_visible {
                // Clear the last rendered pixels once the active trail has
                // fully faded; otherwise the surface would keep its final
                // nearly transparent frame until another input event.
                if self.last_trail_frame_at.take().is_some() {
                    self.trail_only_redraw = true;
                    self.redraw(qh);
                }
                return;
            }
            let frame_due = self
                .last_trail_frame_at
                .is_none_or(|last| now.saturating_duration_since(last) >= FRAME_INTERVAL);
            if frame_due {
                self.last_trail_frame_at = Some(now);
                self.trail_only_redraw = true;
                self.redraw(qh);
            }
            return;
        }
        let Some(points) = self.fading_swipe_trail.as_mut() else {
            self.last_trail_frame_at = None;
            return;
        };
        points.retain(|(_, _, at)| now.saturating_duration_since(*at) < TRAIL_LIFETIME);
        if points.is_empty() {
            self.fading_swipe_trail = None;
            self.last_trail_frame_at = None;
            self.trail_only_redraw = true;
            self.redraw(qh);
            return;
        }
        if self
            .last_trail_frame_at
            .is_some_and(|last| now.saturating_duration_since(last) < FRAME_INTERVAL)
        {
            return;
        }
        self.last_trail_frame_at = Some(now);
        self.trail_only_redraw = true;
        self.redraw(qh);
    }

    pub fn update_hold_preview(&mut self, qh: &QueueHandle<Self>) {
        // Secondary-key preview popup; disabled via `"preview_enabled": false`.
        if self.swipe_traces.values().any(SwipeTrace::qualifies) {
            if self.hold_preview.take().is_some() {
                self.redraw(qh);
            }
            return;
        }
        if self.layout.preview_enabled == Some(false) {
            if self.hold_preview.is_some() {
                self.hold_preview = None;
                self.redraw(qh);
            }
            return;
        }
        if let Some((r_idx, k_idx)) = self.pressed_key
            && let Some(row) = self.layout.rows.get(r_idx)
            && let Some(key) = row.keys.get(k_idx)
            && let Some(sec) = &key.secondary_label
            && let Some(instant) = self.press_instant
            && instant.elapsed().as_millis() > 300
        {
            if self.hold_preview.is_none()
                && let Some((rect, _)) = self.key_rects.get(r_idx).and_then(|r| r.get(k_idx))
            {
                let popup_w = rect.w * 1.25;
                // Taller preview bubble so it clears the user's finger completely.
                // Overridable via the `"preview_height"` layout option.
                let popup_h = self
                    .layout
                    .preview_height
                    .unwrap_or((rect.h * 1.65).max(115.0));
                let mut popup_x = rect.x + rect.w / 2.0 - popup_w / 2.0;
                // Anchor popup bottom to float above the user's touch point with ample clearance
                let mut popup_y = rect.y + 12.0 - popup_h;
                popup_y = popup_y.clamp(2.0, self.height as f32 - popup_h - 4.0);
                popup_x = popup_x.clamp(4.0, self.width as f32 - popup_w - 4.0);
                self.hold_preview = Some(HoldPreview {
                    x: popup_x,
                    y: popup_y,
                    w: popup_w,
                    h: popup_h,
                    label: sec.clone(),
                });
                self.redraw(qh);
            }
            return;
        }
        if self.hold_preview.is_some() {
            self.hold_preview = None;
            self.redraw(qh);
        }
    }

    fn hit_test_at<'a>(
        layout: &'a KeyboardLayout,
        key_rects: &[Vec<(Rect, usize)>],
        clipboard_mode: bool,
        width: u32,
        height: u32,
        theme: &Theme,
        x: f64,
        y: f64,
    ) -> Option<(usize, usize, &'a crate::layout::key::Key)> {
        let clipboard_viewport = if clipboard_mode {
            RenderEngine::clipboard_scroll_metrics(layout, width, height, theme)
                .map(|(top, height, _, _)| Rect {
                    x: 0.0,
                    y: top,
                    w: width as f32,
                    h: height,
                })
        } else {
            None
        };
        RenderEngine::hit_test_with_clip(layout, key_rects, clipboard_viewport, x, y)
    }

    fn key_rect(&self, r_idx: usize, k_idx: usize) -> Option<Rect> {
        self.key_rects
            .get(r_idx)?
            .iter()
            .find(|(_, index)| *index == k_idx)
            .map(|(rect, _)| *rect)
    }

    fn begin_swipe_trace(&mut self, touch_id: i32, r_idx: usize, k_idx: usize, x: f64, y: f64) {
        #[cfg(feature = "futo-swipe")]
        self.invalidate_swipe_preview();
        if self.clipboard_mode || !self.swipe_traces.is_empty() {
            return;
        }
        let Some(rect) = self.key_rect(r_idx, k_idx) else {
            return;
        };
        let threshold = (rect.w as f64 * 0.5).clamp(18.0, 72.0);
        self.swipe_traces
            .insert(touch_id, SwipeTrace::new(x, y, Instant::now(), threshold));
    }

    fn track_swipe_motion(&mut self, touch_id: i32, x: f64, y: f64) {
        if let Some(trace) = self.swipe_traces.get_mut(&touch_id) {
            trace.push(x, y, Instant::now());
        }
    }

    /// Letter-key geometry in normalized letter-area coordinates,
    /// plus (aspect, x0, y0, width, height) for normalising the finger path.
    fn swipe_layout(&self) -> Option<(Vec<SwipeKeyGeom>, f32, f32, f32, f32, f32)> {
        let mut keys: Vec<(char, Rect)> = Vec::new();
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;

        for (r_idx, row) in self.layout.rows.iter().enumerate() {
            let Some(rects) = self.key_rects.get(r_idx) else {
                continue;
            };
            for (rect, k_idx) in rects {
                let Some(key) = row.keys.get(*k_idx) else {
                    continue;
                };
                let KeyAction::Text(text) = &key.action else {
                    continue;
                };
                let mut chars = text.chars();
                let Some(letter) = chars.next() else {
                    continue;
                };
                if chars.next().is_some() || !letter.is_ascii_alphabetic() {
                    continue;
                }
                let letter = letter.to_ascii_lowercase();
                if keys.iter().any(|(existing, _)| *existing == letter) {
                    continue;
                }
                keys.push((letter, *rect));
                min_x = min_x.min(rect.x);
                min_y = min_y.min(rect.y);
                max_x = max_x.max(rect.x + rect.w);
                max_y = max_y.max(rect.y + rect.h);
            }
        }

        if keys.len() < 2 || max_x <= min_x || max_y <= min_y {
            return None;
        }
        let width = (max_x - min_x).max(1.0);
        let height = (max_y - min_y).max(1.0);
        let geometry = keys
            .into_iter()
            .map(|(letter, rect)| SwipeKeyGeom {
                letter,
                cx: (rect.x + rect.w * 0.5 - min_x) / width,
                cy: (rect.y + rect.h * 0.5 - min_y) / height,
                hw: (rect.w * 0.5) / width,
                hh: (rect.h * 0.5) / height,
            })
            .collect();
        Some((geometry, height / width, min_x, min_y, width, height))
    }

    #[cfg(feature = "futo-swipe")]
    fn swipe_decode_input(
        &self,
        trace: &SwipeTrace,
    ) -> Option<(Vec<SwipeKeyGeom>, f32, Vec<(f32, f32, f64)>)> {
        let (keys, aspect, x0, y0, width, height) = self.swipe_layout()?;
        let first_time = trace.points.first().map_or(trace.start, |point| point.2);
        let points = trace
            .points
            .iter()
            .map(|(x, y, at)| {
                (
                    ((*x as f32) - x0) / width,
                    ((*y as f32) - y0) / height,
                    at.duration_since(first_time).as_secs_f64() * 1000.0,
                )
            })
            .collect();
        Some((keys, aspect, points))
    }

    fn recognize_swipe(&mut self, trace: &SwipeTrace) -> Vec<String> {
        let Some((keys, aspect, x0, y0, width, height)) = self.swipe_layout() else {
            return Vec::new();
        };
        if self.swipe_engine.is_none() {
            self.swipe_engine = SwipeRecognizer::new();
            if self.swipe_engine.is_none() {
                tracing::warn!("swipe engine allocation failed");
                return Vec::new();
            }
        }
        #[cfg(feature = "futo-swipe")]
        if let Some(engine) = self.swipe_engine.as_mut() {
            engine.prepare_layout(&keys);
        }

        #[cfg(not(feature = "futo-swipe"))]
        {
            let need_words = self
                .swipe_engine
                .as_mut()
                .map(|engine| engine.prepare_layout(&keys))
                .unwrap_or(false);
            if need_words {
                let words = self.suggest_engine.dictionary.all_words(usize::MAX);
                tracing::info!("geometric swipe engine loading {} base words", words.len());
                if let Some(engine) = self.swipe_engine.as_mut()
                    && !engine.load_words(&words)
                {
                    tracing::warn!("geometric swipe engine could not load dictionary");
                    return Vec::new();
                }
            }
        }

        let first_time = trace.points.first().map_or(trace.start, |point| point.2);
        let points: Vec<(f32, f32, f64)> = trace
            .points
            .iter()
            .map(|(x, y, at)| {
                (
                    ((*x as f32) - x0) / width,
                    ((*y as f32) - y0) / height,
                    at.duration_since(first_time).as_secs_f64() * 1000.0,
                )
            })
            .collect();
        #[cfg(feature = "futo-swipe")]
        {
            let dictionary = &self.suggest_engine.dictionary;
            self.swipe_engine
                .as_mut()
                .map(|engine| engine.decode(&points, dictionary, aspect, 6))
                .unwrap_or_default()
        }
        #[cfg(not(feature = "futo-swipe"))]
        {
            self.swipe_engine
                .as_mut()
                .map(|engine| engine.decode(&points, aspect, 6))
                .unwrap_or_default()
        }
    }

    fn finish_swipe(&mut self, trace: &SwipeTrace, qh: &QueueHandle<Self>) -> bool {
        #[cfg(feature = "futo-swipe")]
        self.invalidate_swipe_preview();
        self.swipe_preview = None;
        let candidates = self.recognize_swipe(trace);
        let Some(best) = candidates.first() else {
            return false;
        };
        let best = self.swipe_case_word(best);
        let left = candidates
            .get(1)
            .map(|word| self.swipe_case_word(word))
            .unwrap_or_else(|| best.clone());
        let right = candidates
            .get(2)
            .map(|word| self.swipe_case_word(word))
            .unwrap_or_else(|| left.clone());
        let slots = vec![left, best, right];
        let best = slots[1].clone();
        tracing::info!(candidate_bytes = best.len(), "swipe candidate recognized");
        self.suggest_engine.candidates = slots;
        self.suggest_engine.is_next_word_mode = false;
        let suggestion_key = self
            .layout
            .rows
            .iter()
            .enumerate()
            .find_map(|(r_idx, row)| {
                row.keys
                    .iter()
                    .position(|key| matches!(key.action, KeyAction::Suggestion(1)))
                    .map(|k_idx| (r_idx, k_idx))
            });
        if let Some((r_idx, k_idx)) = suggestion_key {
            self.handle_key_press(r_idx, k_idx, qh, -1);
            self.pressed_keys.retain(|&key| key != (r_idx, k_idx));
            self.pressed_key = self.pressed_keys.last().copied();
            self.press_instant = None;
            self.hold_preview = None;
            true
        } else {
            false
        }
    }

    fn swipe_case_word(&self, word: &str) -> String {
        if self.caps_lock {
            return word.to_uppercase();
        }
        if self.current_layer == LayerId::Upper {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                return first.to_uppercase().collect::<String>() + chars.as_str();
            }
        }
        word.to_string()
    }

    pub fn handle_motion(&mut self, x: f64, y: f64, qh: &QueueHandle<Self>) {
        // Clipboard swipe scroll: vertical drag moves the card grid. Once the
        // tap threshold is exceeded the pending tap is treated as a scroll.
        if self.clipboard_mode
            && let Some((_, start_y, start_scroll)) = self.clipboard_drag_start
        {
            let dy = (start_y - y) as f32;
            if dy.abs() > 6.0 {
                self.clipboard_dragging = true;
            }
            if self.clipboard_dragging {
                let max = self.clipboard_max_scroll();
                self.clipboard_scroll_y = (start_scroll + dy).clamp(0.0, max.max(0.0));
                self.update_key_rects();
                self.redraw(qh);
                return;
            }
        }
        if let Some((start_x, _)) = self.space_touch_start {
            let delta = x - self.space_last_x;
            let step_threshold = 18.0;
            let steps = (delta.abs() / step_threshold).floor() as usize;

            if steps > 0 {
                self.is_space_swiping = true;
                let right = delta > 0.0;
                for _ in 0..steps {
                    if right {
                        self.send_arrow_right(qh);
                    } else {
                        self.send_arrow_left(qh);
                    }
                }
                self.space_last_x += step_threshold * steps as f64 * delta.signum();
                tracing::info!(steps, right, "spacebar swipe moved caret");
            }

            self.swipe_offset = Some((x - start_x) as f32);
            self.redraw(qh);
        }
    }

    pub fn send_text(&mut self, text: &str) {
        let has_mods = ((self.pending_mods | self.held_mods) & !Self::MOD_SHIFT) != 0;
        let source = self.active_text_input_source();
                tracing::info!(
            text_bytes = text.len(),
            source = source.label_with_readback(self.config.behavior.surrounding_text_readback),
            has_mods,
            surrounding_bytes = self.im_state.surrounding_text.as_ref().map_or(0, |text| text.len()),
            cursor = self.im_state.cursor_pos,
            anchor = self.im_state.anchor_pos,
            "send_text"
        );
        if source == TextInputSource::VirtualKeyboard && !self.fallback_warning_emitted {
            tracing::warn!(
                "Using virtual-keyboard fallback: text can be inserted, but existing app text and selections are unreadable"
            );
            self.fallback_warning_emitted = true;
        }
        let use_im = source == TextInputSource::InputMethodV2 && !has_mods;
        if use_im && let Some(ref im) = self.input_method {
            im.commit_string(text.to_string());
            im.commit(self.im_state.serial);
            tracing::info!("send_text via commit_string");
            return;
        }
        tracing::info!("send_text via vk fallback");

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

    /// Keep the cached input-method buffer aligned with a backspace emitted
    /// through the virtual keyboard while the compositor echo is in flight.
    pub fn pop_surrounding_char(&mut self) {
        let cursor = self.im_state.cursor_pos as usize;
        let anchor = self.im_state.anchor_pos as usize;
        let Some(ref mut text) = self.im_state.surrounding_text else {
            return;
        };
        let cursor = text.floor_char_boundary(cursor.min(text.len()));
        let anchor = text.floor_char_boundary(anchor.min(text.len()));

        if cursor != anchor {
            let start = cursor.min(anchor);
            let end = cursor.max(anchor);
            if text.is_char_boundary(start) && text.is_char_boundary(end) {
                text.replace_range(start..end, "");
                self.im_state.cursor_pos = start as u32;
                self.im_state.anchor_pos = start as u32;
            }
        } else if cursor > 0 {
            let previous = text[..cursor]
                .char_indices()
                .last()
                .map_or(0, |(index, _)| index);
            text.drain(previous..cursor);
            self.im_state.cursor_pos = previous as u32;
            self.im_state.anchor_pos = previous as u32;
        }
    }

    pub fn send_enter(&mut self) {
        self.send_keycode(KEY_ENTER);
    }

    pub fn send_tab(&mut self) {
        self.send_keycode(KEY_TAB);
    }

    pub fn send_space(&mut self) {
        if self.active_text_input_source() == TextInputSource::InputMethodV2 {
            self.send_text(" ");
        } else {
            self.send_keycode(KEY_SPACE);
        }
    }

    pub fn send_arrow_left(&mut self, qh: &QueueHandle<Self>) {
        self.send_keycode(KEY_LEFT);
        self.sync_horizontal_caret(false, qh);
    }

    pub fn send_arrow_right(&mut self, qh: &QueueHandle<Self>) {
        self.send_keycode(KEY_RIGHT);
        self.sync_horizontal_caret(true, qh);
    }

    fn sync_horizontal_caret(&mut self, right: bool, qh: &QueueHandle<Self>) {
        let Some(text) = self.im_state.surrounding_text.clone() else {
            self.suggest_engine.clear();
            self.sync_layout_and_redraw(qh);
            return;
        };
        let cursor = self.im_state.cursor_pos as usize;
        let anchor = self.im_state.anchor_pos as usize;
        if cursor > text.len()
            || anchor > text.len()
            || !text.is_char_boundary(cursor)
            || !text.is_char_boundary(anchor)
        {
            self.suggest_engine.clear();
            self.sync_layout_and_redraw(qh);
            return;
        }

        let next = if cursor != anchor {
            if right { cursor.max(anchor) } else { cursor.min(anchor) }
        } else if right {
            text[cursor..]
                .chars()
                .next()
                .map_or(cursor, |ch| cursor + ch.len_utf8())
        } else {
            text[..cursor]
                .char_indices()
                .last()
                .map_or(0, |(index, _)| index)
        };
        self.im_state.cursor_pos = next as u32;
        self.im_state.anchor_pos = next as u32;
        self.sync_suggest_with_surrounding(&text, next as u32, true, qh);
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
            let pending = self.pending_mods;
            let held = self.held_mods;
            let shift_bit = if shift { Self::MOD_SHIFT } else { 0 };
            let depressed = pending | held | shift_bit;
            // Held modifiers are already physically down (pressed at mod-down
            // time); only transient one-shot/shift bits need press/release here.
            let transient = (pending | shift_bit) & !held;

            // Any emission while modifiers are held counts as a chord: held
            // bits survive instead of converting to a tap-latch on release.
            if held != 0 {
                self.held_mod_chorded |= held;
            }

            if depressed != 0 {
                vk.modifiers(depressed, 0, 0, 0);

                // Press down physical modifier keycodes so compositors (Hyprland) and client apps
                // (GTK, Qt, Chromium, terminals) register actual shortcut combinations
                // rather than inserting raw characters into input fields.
                if transient & Self::MOD_CTRL != 0 {
                    vk.key(time, KEY_LEFTCTRL, wl_keyboard::KeyState::Pressed.into());
                }
                if transient & Self::MOD_ALT != 0 {
                    vk.key(time, KEY_LEFTALT, wl_keyboard::KeyState::Pressed.into());
                }
                if transient & Self::MOD_SUPER != 0 {
                    vk.key(time, KEY_LEFTMETA, wl_keyboard::KeyState::Pressed.into());
                }
                if transient & Self::MOD_SHIFT != 0 {
                    vk.key(time, KEY_LEFTSHIFT, wl_keyboard::KeyState::Pressed.into());
                }
            }

            vk.key(time, keycode, wl_keyboard::KeyState::Pressed.into());
            vk.key(time, keycode, wl_keyboard::KeyState::Released.into());

            if depressed != 0 {
                if transient & Self::MOD_SHIFT != 0 {
                    vk.key(time, KEY_LEFTSHIFT, wl_keyboard::KeyState::Released.into());
                }
                if transient & Self::MOD_SUPER != 0 {
                    vk.key(time, KEY_LEFTMETA, wl_keyboard::KeyState::Released.into());
                }
                if transient & Self::MOD_ALT != 0 {
                    vk.key(time, KEY_LEFTALT, wl_keyboard::KeyState::Released.into());
                }
                if transient & Self::MOD_CTRL != 0 {
                    vk.key(time, KEY_LEFTCTRL, wl_keyboard::KeyState::Released.into());
                }

                if pending != 0 {
                    self.pending_mods = 0;
                }
                // Restore held-modifier state (never drop to zero while held).
                vk.modifiers(held, 0, 0, 0);
                if pending != 0 {
                    tracing::info!("consumed one-shot mods {:b}", pending);
                }
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
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _scale: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _transform: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }
    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for WaylandState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
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
    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            let _ = self.seat_state.get_pointer(qh, &seat);
        }
        if capability == Capability::Touch {
            let _ = self.seat_state.get_touch(qh, &seat);
        }
    }
    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        _capability: Capability,
    ) {
    }
    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
    }
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
                    self.note_input(false);
                    let (x, y) = event.position;
                    if self.clipboard_mode {
                        self.clipboard_drag_start = Some((x, y, self.clipboard_scroll_y));
                        self.clipboard_dragging = false;
                    }
                    if let Some((r_idx, k_idx, key)) =
                        Self::hit_test_at(
                            &self.layout,
                            &self.key_rects,
                            self.clipboard_mode,
                            self.width,
                            self.height,
                            &self.theme,
                            x,
                            y,
                        )
                    {
                        // Defer clipboard pastes to release so a vertical swipe
                        // scrolls instead of pasting.
                        if self.clipboard_mode && matches!(key.action, KeyAction::ClipboardItem(_))
                        {
                            self.pressed_key = Some((r_idx, k_idx));
                            if !self.pressed_keys.contains(&(r_idx, k_idx)) {
                                self.pressed_keys.push((r_idx, k_idx));
                            }
                            self.redraw(qh);
                            continue;
                        }
                        if matches!(key.action, KeyAction::Space) {
                            self.space_touch_start = Some((x, y));
                            self.space_last_x = x;
                            self.is_space_swiping = false;
                        }
                        if matches!(key.action, KeyAction::Text(_)) {
                            self.last_shift_press = None;
                            self.pressed_key = Some((r_idx, k_idx));
                            self.press_instant = Some(Instant::now());
                            if !self.pressed_keys.contains(&(r_idx, k_idx)) {
                                self.pressed_keys.push((r_idx, k_idx));
                            }
                            self.pointer_pending_detail = Some((
                                key.action.clone(),
                                key.secondary_label.clone(),
                                Instant::now(),
                            ));
                            self.redraw(qh);
                        } else {
                            self.handle_key_press(r_idx, k_idx, qh, Self::POINTER_TOUCH_ID);
                        }
                    }
                }
                PointerEventKind::Release { .. } => {
                    let was_clipboard_drag = self.clipboard_dragging;
                    self.clipboard_drag_start = None;
                    self.clipboard_dragging = false;
                    // Deferred clipboard tap: paste only when it was a tap,
                    // not a scroll swipe.
                    if self.clipboard_mode
                        && let Some((r_idx, k_idx)) = self.pressed_key
                    {
                        let is_clip = self
                            .layout
                            .rows
                            .get(r_idx)
                            .and_then(|r| r.keys.get(k_idx))
                            .map(|k| matches!(k.action, KeyAction::ClipboardItem(_)))
                            .unwrap_or(false);
                        if is_clip {
                            self.pressed_keys.retain(|&p| p != (r_idx, k_idx));
                            self.pressed_key = self.pressed_keys.last().copied();
                            if !was_clipboard_drag {
                                self.handle_key_press(r_idx, k_idx, qh, Self::POINTER_TOUCH_ID);
                            } else {
                                self.sync_layout_and_redraw(qh);
                            }
                            continue;
                        }
                    }
                    if let Some((action, secondary_label, down_time)) =
                        self.pointer_pending_detail.take()
                    {
                        let elapsed = down_time.elapsed();
                        let is_long_press = elapsed.as_millis() >= 300;
                        let has_command_mods = ((self.pending_mods | self.held_mods)
                            & (Self::MOD_CTRL | Self::MOD_ALT | Self::MOD_SUPER))
                            != 0;
                        if is_long_press && let Some(sec) = secondary_label {
                            if !has_command_mods {
                                for ch in sec.chars() {
                                    self.suggest_engine.push_char(ch);
                                }
                            }
                            self.send_text(&sec);
                        } else if let KeyAction::Text(primary) = action {
                            if !has_command_mods {
                                for ch in primary.chars() {
                                    self.suggest_engine.push_char(ch);
                                }
                            }
                            self.send_text(&primary);
                        }
                        if !self.caps_lock
                            && self.current_layer == LayerId::Upper
                            && (self.held_mods & Self::MOD_SHIFT) == 0
                        {
                            self.current_layer = LayerId::Lower;
                            self.pending_mods &= !Self::MOD_SHIFT;
                            self.sync_mods();
                        }
                        self.pressed_keys.clear();
                        self.pressed_key = None;
                        self.press_instant = None;
                        self.hold_preview = None;
                        self.repeat_key = None;
                        self.repeat_start = None;
                        self.last_repeat_at = None;
                        self.sync_layout_and_redraw(qh);
                    } else if let Some((r_idx, k_idx)) = self.pressed_key {
                        // Ownership-first: a pointer lifting off a held
                        // modifier releases it via mod_up, never generically.
                        let owned = self.mods_owned_by(Self::POINTER_TOUCH_ID);
                        if !owned.is_empty() {
                            self.pressed_keys.retain(|&p| p != (r_idx, k_idx));
                            self.pressed_key = self.pressed_keys.last().copied();
                            for mask in owned {
                                self.mod_up(mask, Self::POINTER_TOUCH_ID, qh);
                            }
                            self.sync_layout_and_redraw(qh);
                            continue;
                        }
                        let mask = self
                            .layout
                            .rows
                            .get(r_idx)
                            .and_then(|r| r.keys.get(k_idx))
                            .map(|k| Self::mod_mask_for_action(&k.action))
                            .unwrap_or(0);
                        if mask != 0 {
                            self.pressed_keys.retain(|&p| p != (r_idx, k_idx));
                            self.pressed_key = self.pressed_keys.last().copied();
                            self.mod_up(mask, Self::POINTER_TOUCH_ID, qh);
                            self.sync_layout_and_redraw(qh);
                            continue;
                        }
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
        self.note_input(true);
        let (x, y) = position;
        tracing::info!(
            "touch down id={} pos=({:.0},{:.0}) active={}",
            _id,
            x,
            y,
            self.active_touches.len() + 1
        );
        // A second finger turns the gesture into a tap/chord, not a swipe.
        self.swipe_traces.clear();
        #[cfg(feature = "futo-swipe")]
        self.invalidate_swipe_preview();
        if self.clipboard_mode {
            self.clipboard_drag_start = Some((x, y, self.clipboard_scroll_y));
            self.clipboard_dragging = false;
        }
        if let Some((r_idx, k_idx, key)) =
            Self::hit_test_at(
                &self.layout,
                &self.key_rects,
                self.clipboard_mode,
                self.width,
                self.height,
                &self.theme,
                x,
                y,
            )
        {
            tracing::info!("  -> hit key {:?} at {}:{}", key.action, r_idx, k_idx);
            let swipe_start = self.active_touches.is_empty()
                && matches!(
                    &key.action,
                    KeyAction::Text(text)
                        if text.chars().count() == 1
                            && text
                                .chars()
                                .next()
                                .is_some_and(|ch| ch.is_ascii_alphabetic())
                );
            // Defer clipboard pastes to release so a vertical swipe scrolls
            // instead of pasting.
            if self.clipboard_mode && matches!(key.action, KeyAction::ClipboardItem(_)) {
                self.active_touches.insert(_id, (r_idx, k_idx));
                self.touch_press_feedback.insert(
                    (r_idx, k_idx),
                    (
                        key.action.clone(),
                        Instant::now() + TOUCH_PRESS_FEEDBACK_DURATION,
                    ),
                );
                if !self.pressed_keys.contains(&(r_idx, k_idx)) {
                    self.pressed_keys.push((r_idx, k_idx));
                }
                self.pressed_key = Some((r_idx, k_idx));
                self.redraw(qh);
                return;
            }
            if matches!(key.action, KeyAction::Space) {
                self.space_touch_start = Some((x, y));
                self.space_last_x = x;
                self.is_space_swiping = false;
            }
            self.active_touches.insert(_id, (r_idx, k_idx));
            self.touch_press_feedback.insert(
                (r_idx, k_idx),
                (
                    key.action.clone(),
                    Instant::now() + TOUCH_PRESS_FEEDBACK_DURATION,
                ),
            );
            if matches!(key.action, KeyAction::Text(_)) {
                self.last_shift_press = None;
                self.pending_taps.insert(_id, (r_idx, k_idx));
                self.pending_tap_details.insert(
                    _id,
                    (
                        key.action.clone(),
                        key.secondary_label.clone(),
                        Instant::now(),
                    ),
                );
                if !self.pressed_keys.contains(&(r_idx, k_idx)) {
                    self.pressed_keys.push((r_idx, k_idx));
                }
                self.pressed_key = Some((r_idx, k_idx));
                self.press_instant = Some(Instant::now());
                self.redraw(qh);
                if swipe_start {
                    self.begin_swipe_trace(_id, r_idx, k_idx, x, y);
                }
            } else {
                self.pressed_key = Some((r_idx, k_idx));
                if !self.pressed_keys.contains(&(r_idx, k_idx)) {
                    self.pressed_keys.push((r_idx, k_idx));
                }
                self.redraw(qh);
                self.handle_key_press(r_idx, k_idx, qh, _id);
            }
        } else {
            tracing::info!("  -> miss (no key)");
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
        tracing::info!(
            "touch up id={} active_before={}",
            _id,
            self.active_touches.len()
        );
        let details = self.pending_tap_details.remove(&_id);
        if let Some(trace) = self.swipe_traces.remove(&_id)
            && trace.qualifies()
        {
            self.fading_swipe_trail = Some(trace.points.clone());
            self.last_trail_frame_at = None;
            // Suppress the pending letter tap: this release is a word swipe.
            if let Some(pending) = self.pending_taps.remove(&_id) {
                self.pressed_keys.retain(|&p| p != pending);
            }
            self.active_touches.remove(&_id);
            self.pressed_key = self.pressed_keys.last().copied();
            self.press_instant = None;
            self.hold_preview = None;
            if !self.finish_swipe(&trace, qh) {
                tracing::info!("swipe gesture produced no word");
            }
            if !self.caps_lock
                && self.current_layer == LayerId::Upper
                && (self.held_mods & Self::MOD_SHIFT) == 0
            {
                self.current_layer = LayerId::Lower;
                self.pending_mods &= !Self::MOD_SHIFT;
                self.sync_mods();
            }
            self.sync_layout_and_redraw(qh);
            return;
        }
        if let Some(pending) = self.pending_taps.remove(&_id) {
            self.active_touches.remove(&_id);
            self.pressed_keys.retain(|&p| p != pending);
            self.pressed_key = self.pressed_keys.last().copied();
            if let Some((action, secondary_label, down_time)) = details {
                let elapsed = down_time.elapsed();
                let is_long_press = elapsed.as_millis() >= 300;

                let has_command_mods = ((self.pending_mods | self.held_mods)
                    & (Self::MOD_CTRL | Self::MOD_ALT | Self::MOD_SUPER))
                    != 0;
                if is_long_press && let Some(sec) = secondary_label {
                    if !has_command_mods {
                        for ch in sec.chars() {
                            self.suggest_engine.push_char(ch);
                        }
                    }
                    self.send_text(&sec);
                } else if let KeyAction::Text(primary) = action {
                    if !has_command_mods {
                        for ch in primary.chars() {
                            self.suggest_engine.push_char(ch);
                        }
                    }
                    self.send_text(&primary);
                }

                if !self.caps_lock
                    && self.current_layer == LayerId::Upper
                    && (self.held_mods & Self::MOD_SHIFT) == 0
                {
                    self.current_layer = LayerId::Lower;
                    self.pending_mods &= !Self::MOD_SHIFT;
                    self.sync_mods();
                }
                self.hold_preview = None;
                self.press_instant = None;
                self.sync_layout_and_redraw(qh);
                return;
            }
        }
        if let Some((r_idx, k_idx)) = self.active_touches.remove(&_id) {
            // Ownership-first modifier release: a finger lifting off a key it
            // holds as a modifier releases that modifier even if a layer
            // switch rebuilt the layout under the finger since press-down.
            let owned = self.mods_owned_by(_id);
            if !owned.is_empty() {
                self.pressed_keys.retain(|&p| p != (r_idx, k_idx));
                self.pressed_key = self.pressed_keys.last().copied();
                for mask in owned {
                    self.mod_up(mask, _id, qh);
                }
                self.sync_layout_and_redraw(qh);
                return;
            }
            let was_clipboard_drag = self.clipboard_dragging;
            // Deferred clipboard tap: paste only on tap, not on scroll swipe.
            let is_clip = self
                .layout
                .rows
                .get(r_idx)
                .and_then(|r| r.keys.get(k_idx))
                .map(|k| matches!(k.action, KeyAction::ClipboardItem(_)))
                .unwrap_or(false);
            if self.clipboard_mode && is_clip {
                self.pressed_keys.retain(|&p| p != (r_idx, k_idx));
                self.pressed_key = self.pressed_keys.last().copied();
                self.clipboard_drag_start = None;
                self.clipboard_dragging = false;
                if !was_clipboard_drag {
                    self.handle_key_press(r_idx, k_idx, qh, _id);
                } else {
                    self.sync_layout_and_redraw(qh);
                }
                return;
            }
            self.handle_key_release(r_idx, k_idx, qh);
        } else if let Some((r_idx, k_idx)) = self.pressed_key {
            // Fallback for releases without a matching touch-down: still
            // route held modifiers through mod_up so they can't stick.
            let mask = self
                .layout
                .rows
                .get(r_idx)
                .and_then(|r| r.keys.get(k_idx))
                .map(|k| Self::mod_mask_for_action(&k.action))
                .unwrap_or(0);
            if mask != 0 {
                self.pressed_keys.retain(|&p| p != (r_idx, k_idx));
                self.pressed_key = self.pressed_keys.last().copied();
                self.mod_up(mask, _id, qh);
                self.sync_layout_and_redraw(qh);
            } else {
                self.handle_key_release(r_idx, k_idx, qh);
            }
        }
        self.clipboard_drag_start = None;
        self.clipboard_dragging = false;
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
        self.track_swipe_motion(_id, x, y);
        let swipe_qualifies = self
            .swipe_traces
            .get(&_id)
            .is_some_and(SwipeTrace::qualifies);
        #[cfg(feature = "futo-swipe")]
        let needs_swipe_suppression_redraw = swipe_qualifies
            && (self.pressed_key.is_some()
                || self.hold_preview.is_some()
                || self.pending_taps.contains_key(&_id));
        if swipe_qualifies {
            if let Some((r, k)) = self.pending_taps.remove(&_id) {
                self.pressed_keys.retain(|&pressed| pressed != (r, k));
            }
            self.pressed_key = self.pressed_keys.last().copied();
            self.press_instant = None;
            self.hold_preview = None;
        }
        let preview_trace = self.swipe_traces.get_mut(&_id).and_then(|trace| {
            if trace.qualifies() && trace.preview_due(Instant::now()) {
                Some(trace.clone())
            } else {
                None
            }
        });
        if let Some(trace) = preview_trace {
            if let Some((r, k)) = self.pending_taps.remove(&_id) {
                self.pressed_keys.retain(|&pressed| pressed != (r, k));
            }
            self.pressed_key = self.pressed_keys.last().copied();
            self.press_instant = None;
            #[cfg(feature = "futo-swipe")]
            {
                self.queue_swipe_preview(_id, &trace);
                if needs_swipe_suppression_redraw {
                    self.sync_layout_and_redraw(qh);
                }
            }
            #[cfg(not(feature = "futo-swipe"))]
            {
                let words = self.recognize_swipe(&trace);
                if let Some(best) = words.first() {
                    let best = self.swipe_case_word(best);
                    if let Some(active) = self.swipe_traces.get_mut(&_id) {
                        active.candidates = vec![best.clone()];
                    }
                    self.swipe_preview = Some(best);
                } else {
                    if let Some(active) = self.swipe_traces.get_mut(&_id) {
                        active.candidates.clear();
                    }
                    self.swipe_preview = None;
                }
                self.sync_layout_and_redraw(qh);
            }
        }
        self.handle_motion(x, y, qh);
    }

    fn cancel(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, _touch: &wl_touch::WlTouch) {
        // Release held modifiers first so no physical key or latch can stick.
        self.release_all_held_mods();
        self.pressed_key = None;
        self.pressed_keys.clear();
        self.touch_press_feedback.clear();
        self.active_touches.clear();
        self.pending_taps.clear();
        self.pending_tap_details.clear();
        self.swipe_preview = None;
        self.fading_swipe_trail = None;
        self.swipe_traces.clear();
        #[cfg(feature = "futo-swipe")]
        self.invalidate_swipe_preview();
        self.pointer_pending_detail = None;
        self.press_instant = None;
        self.hold_preview = None;
        self.repeat_key = None;
        self.repeat_start = None;
        self.last_repeat_at = None;
        self.space_touch_start = None;
        self.is_space_swiping = false;
        self.swipe_offset = None;
        self.clipboard_drag_start = None;
        self.clipboard_dragging = false;
        self.redraw(qh);
    }

    fn shape(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _major: f64,
        _minor: f64,
    ) {
    }
    fn orientation(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _orientation: f64,
    ) {
    }
}

/// Return backspaces and an optional text range for a suggestion replacement.
fn token_range_around_cursor(
    text: &str,
    cursor_pos: u32,
    anchor_pos: u32,
) -> Option<(usize, usize, usize, usize, usize)> {
    let cursor = text.floor_char_boundary((cursor_pos as usize).min(text.len()));
    let anchor = text.floor_char_boundary((anchor_pos as usize).min(text.len()));
    if cursor != anchor {
        return None;
    }
    let start = text[..cursor]
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_alphabetic() && *ch != '\'')
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    let end = text[cursor..]
        .char_indices()
        .find(|(_, ch)| !ch.is_alphabetic() && *ch != '\'')
        .map_or(text.len(), |(index, _)| cursor + index);
    if start >= cursor || cursor >= end {
        return None;
    }
    Some((
        start,
        end,
        cursor,
        text[start..cursor].chars().count(),
        text[cursor..end].chars().count(),
    ))
}

pub fn calculate_replacement_target(
    current_word: &str,
    surrounding_text: Option<&str>,
    cursor_pos: u32,
    anchor_pos: u32,
) -> (usize, Option<(usize, usize)>) {
    let Some(text) = surrounding_text else {
        return (current_word.chars().count(), None);
    };

    let cursor = text.floor_char_boundary((cursor_pos as usize).min(text.len()));
    let anchor = text.floor_char_boundary((anchor_pos as usize).min(text.len()));
    let start = cursor.min(anchor);
    let end = cursor.max(anchor);

    if start != end {
        // Use whitespace as the token boundary so URLs remain intact.
        let token_start = text[..start]
            .char_indices()
            .rev()
            .find(|(_, ch)| ch.is_whitespace())
            .map_or(0, |(index, ch)| index + ch.len_utf8());
        let prefix_count = text[token_start..start].chars().count();
        return (prefix_count + 1, Some((token_start, end)));
    }

    let before = &text[..cursor];
    if !before.is_empty() && !before.chars().last().is_some_and(char::is_whitespace) {
        let word_start = before
            .char_indices()
            .rev()
            .find(|(_, ch)| !ch.is_alphabetic() && *ch != '\'')
            .map_or(0, |(index, ch)| index + ch.len_utf8());
        let app_word = &before[word_start..];
        // Ignore delayed echoes that are shorter prefixes of the tracked word.
        if !app_word.is_empty()
            && (current_word.starts_with(app_word) || app_word.eq_ignore_ascii_case(current_word))
        {
            return (app_word.chars().count(), Some((word_start, cursor)));
        }
    }

    (current_word.chars().count(), None)
}

/// Return the candidate suffix when it extends the typed prefix.
pub fn candidate_suffix_for_prefix(current_word: &str, chosen_word: &str) -> Option<String> {
    if current_word.is_empty() {
        return None;
    }

    let prefix_len = current_word.chars().count();
    let candidate_prefix = chosen_word.chars().take(prefix_len).collect::<String>();
    if !candidate_prefix.eq_ignore_ascii_case(current_word) {
        return None;
    }

    Some(chosen_word.chars().skip(prefix_len).collect())
}

/// Return the suffix and selected range reported by the host.
pub fn host_completion_suffix(
    surrounding_text: Option<&str>,
    cursor_pos: u32,
    anchor_pos: u32,
    chosen_word: &str,
) -> Option<(String, (usize, usize))> {
    let text = surrounding_text?;
    let cursor = text.floor_char_boundary((cursor_pos as usize).min(text.len()));
    let anchor = text.floor_char_boundary((anchor_pos as usize).min(text.len()));
    if cursor == anchor {
        return None;
    }

    let selection_start = cursor.min(anchor);
    let selection_end = cursor.max(anchor);
    let token_start = text[..selection_start]
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    let typed_prefix = &text[token_start..selection_start];
    let suffix = candidate_suffix_for_prefix(typed_prefix, chosen_word)?;
    Some((suffix, (selection_start, selection_end)))
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

#[cfg(test)]
mod state_tests {
    #[test]
    fn swipe_trace_compaction_preserves_origin_and_latest_point() {
        let start = Instant::now();
        let mut trace = SwipeTrace::new(0.0, 0.0, start, 10.0);
        for i in 1..=600 {
            trace.push(i as f64, 0.0, start + Duration::from_millis(i));
        }
        assert!(trace.points.len() <= 256);
        assert_eq!(trace.points.first().map(|p| (p.0, p.1)), Some((0.0, 0.0)));
        assert_eq!(trace.points.last().map(|p| (p.0, p.1)), Some((600.0, 0.0)));
        assert!(trace.qualifies());
    }

    use super::{SwipeTrace, WaylandState};
    use std::time::{Duration, Instant};

    #[test]
    fn test_is_terminal_class() {
        // Known terminals (matched case-insensitively by caller convention;
        // classes arrive lowercased from active_window_class).
        for class in [
            "com.mitchellh.ghostty",
            "foot",
            "footclient",
            "kitty",
            "alacritty",
            "org.wezfurlong.wezterm",
            "gnome-terminal-server",
            "konsole",
            "xterm",
            "my-terminal-app",
        ] {
            assert!(
                WaylandState::is_terminal_class(class),
                "{class} should count as terminal"
            );
        }
        // GUI apps, browsers, editors go the Ctrl+V route.
        for class in ["firefox", "chromium", "code", "org.gnome.nautilus", ""] {
            assert!(
                !WaylandState::is_terminal_class(class),
                "{class} should not count as terminal"
            );
        }
    }

    #[test]
    fn test_calculate_replacement_target_normal() {
        let (count, range) =
            super::calculate_replacement_target("worl", Some("hello worl"), 10, 10);
        assert_eq!(count, 4);
        assert_eq!(range, Some((6, 10)));
    }

    #[test]
    fn test_calculate_replacement_target_after_partial_backspace() {
        let (count, range) =
            super::calculate_replacement_target("world", Some("hello worl"), 10, 10);
        assert_eq!(count, 4);
        assert_eq!(range, Some((6, 10)));
    }

    #[test]
    fn test_calculate_replacement_target_single_word() {
        let (count, range) = super::calculate_replacement_target("upda", Some("upda"), 4, 4);
        assert_eq!(count, 4);
        assert_eq!(range, Some((0, 4)));
    }

    #[test]
    fn test_calculate_replacement_target_utf8_prefix() {
        let (count, range) = super::calculate_replacement_target("worl", Some("Café worl"), 10, 10);
        assert_eq!(count, 4);
        assert_eq!(range, Some((6, 10)));
    }

    #[test]
    fn test_calculate_replacement_target_without_surrounding_text() {
        let (count, range) = super::calculate_replacement_target("worl", None, 0, 0);
        assert_eq!(count, 4);
        assert_eq!(range, None);
    }

    #[test]
    fn test_calculate_replacement_target_rejects_stale_longer_word() {
        let (count, range) = super::calculate_replacement_target("hel", Some("hello"), 5, 5);
        assert_eq!(count, 3);
        assert_eq!(range, None);
    }

    #[test]
    fn test_calculate_replacement_target_host_autocomplete_anchor_before_cursor() {
        let (count, range) =
            super::calculate_replacement_target("w", Some("www.youtube.com"), 15, 1);
        assert_eq!(count, 2);
        assert_eq!(range, Some((0, 15)));
    }

    #[test]
    fn test_calculate_replacement_target_host_autocomplete_cursor_before_anchor() {
        let (count, range) =
            super::calculate_replacement_target("w", Some("www.youtube.com"), 1, 15);
        assert_eq!(count, 2);
        assert_eq!(range, Some((0, 15)));
    }

    #[test]
    fn test_calculate_replacement_target_host_autocomplete_with_preceding_text() {
        let (count, range) =
            super::calculate_replacement_target("w", Some("visit www.youtube.com"), 21, 7);
        assert_eq!(count, 2);
        assert_eq!(range, Some((6, 21)));
    }

    #[test]
    fn test_host_completion_suffix_keeps_typed_prefix() {
        let result = super::host_completion_suffix(Some("www.youtube.com"), 1, 15, "well");
        assert_eq!(result, Some(("ell".to_string(), (1, 15))));
    }

    #[test]
    fn test_host_completion_suffix_handles_reversed_selection() {
        let result = super::host_completion_suffix(Some("www.youtube.com"), 15, 1, "we");
        assert_eq!(result, Some(("e".to_string(), (1, 15))));
    }

    #[test]
    fn test_host_completion_suffix_rejects_unrelated_candidate() {
        assert_eq!(
            super::host_completion_suffix(Some("www.youtube.com"), 1, 15, "hello"),
            None
        );
    }

    #[test]
    fn test_candidate_suffix_for_prefix_without_surrounding_text() {
        assert_eq!(
            super::candidate_suffix_for_prefix("w", "well"),
            Some("ell".to_string())
        );
        assert_eq!(
            super::candidate_suffix_for_prefix("w", "we"),
            Some("e".to_string())
        );
    }

    #[test]
    fn test_candidate_suffix_for_prefix_rejects_nonmatching_candidate() {
        assert_eq!(super::candidate_suffix_for_prefix("w", "hello"), None);
    }

    #[test]
    fn test_compute_effective_keyboard_height() {
        // When custom_layout specifies a height, it overrides general.height
        assert_eq!(
            super::compute_effective_keyboard_height(Some(470), 360),
            470
        );
        // When custom_layout has no height, it falls back to general.height
        assert_eq!(super::compute_effective_keyboard_height(None, 360), 360);
    }
}
