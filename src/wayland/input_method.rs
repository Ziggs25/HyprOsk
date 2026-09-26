use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::{self, ZwpInputMethodV2},
};
use crate::wayland::state::WaylandState;

#[derive(Debug, Clone, Default)]
pub struct InputMethodState {
    pub is_active: Arc<AtomicBool>,
    pub serial: u32,
    pub surrounding_text: Option<String>,
    pub cursor_pos: u32,
    pub content_hint: u32,
    pub content_purpose: u32,
    pub last_content_purpose: Option<u32>,
}

impl Dispatch<ZwpInputMethodManagerV2, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwpInputMethodManagerV2,
        _event: <ZwpInputMethodManagerV2 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpInputMethodV2, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _proxy: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => {
                tracing::info!("Received text-input Activate event -> Auto-showing HyprOsk");
                state.im_state.is_active.store(true, Ordering::SeqCst);
                state.im_state.last_content_purpose = None;
                state.im_state.surrounding_text = None;
                state.im_state.cursor_pos = 0;
                state.suggest_engine.clear();
                if state.config.behavior.auto_show && state.allow_auto_show() {
                    state.show_keyboard(qh);
                    // If triggered by auto-show, allow auto-hide on blur
                    state.manually_shown = false;
                }
            }
            zwp_input_method_v2::Event::Deactivate => {
                tracing::info!("Received text-input Deactivate event (manually_shown={})", state.manually_shown);
                state.im_state.is_active.store(false, Ordering::SeqCst);
                state.im_state.last_content_purpose = None;
                state.suggest_engine.clear();
                if state.config.behavior.auto_hide && !state.manually_shown {
                    state.hide_keyboard(qh);
                }
            }
            zwp_input_method_v2::Event::SurroundingText { text, cursor, anchor } => {
                let effective_cursor = cursor.max(anchor);
                state.im_state.surrounding_text = Some(text.clone());
                state.im_state.cursor_pos = effective_cursor;
                // Only sync when the field was cleared or no word is actively
                // being typed. While typing (current_word non-empty) Chromium
                // emits lagging SurroundingText echoes (e.g. stale 'b' arriving
                // after 'r' was typed for 'br'); syncing those would clobber
                // current_word back to the stale prefix and cause 'bbro' on
                // replacement. Skipping preserves the typed word; see
                // SuggestEngine::sync_with_text stale-prefix guards.
                if text.is_empty() || state.suggest_engine.current_word.is_empty() {
                    state.sync_suggest_with_surrounding(&text, effective_cursor, qh);
                }
            }
            zwp_input_method_v2::Event::ContentType { hint, purpose } => {
                let p: u32 = purpose.into();
                let h: u32 = hint.into();
                state.im_state.content_hint = h;
                state.im_state.content_purpose = p;
                state.adapt_layout_for_content_purpose(p, qh);
            }
            zwp_input_method_v2::Event::Done => {
                // Batch updates finished
            }
            zwp_input_method_v2::Event::Unavailable => {
                tracing::warn!("zwp_input_method_v2 is unavailable on this seat");
                state.input_method = None;
            }
            _ => {}
        }
    }
}
