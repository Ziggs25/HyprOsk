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
                if state.config.behavior.auto_show {
                    state.show_keyboard(qh);
                }
            }
            zwp_input_method_v2::Event::Deactivate => {
                tracing::info!("Received text-input Deactivate event -> Auto-hiding HyprOsk");
                state.im_state.is_active.store(false, Ordering::SeqCst);
                if state.config.behavior.auto_show {
                    state.hide_keyboard(qh);
                }
            }
            zwp_input_method_v2::Event::SurroundingText { text, cursor, anchor: _ } => {
                state.im_state.surrounding_text = Some(text);
                state.im_state.cursor_pos = cursor;
            }
            zwp_input_method_v2::Event::ContentType { hint, purpose } => {
                state.im_state.content_hint = hint.into();
                state.im_state.content_purpose = purpose.into();
                state.adapt_layout_for_content_purpose(purpose.into(), qh);
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
