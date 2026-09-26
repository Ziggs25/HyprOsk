use crate::wayland::state::WaylandState;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3;
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::{self, ZwpInputMethodV2},
};

/// Identifies how HyprOsk can exchange text with the focused application.
///
/// Wayland does not provide a universal readback channel. The virtual
/// keyboard can write keys to almost any client, while the input-method path
/// additionally exposes surrounding text and selection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextInputSource {
    InputMethodV2,
    #[default]
    VirtualKeyboard,
    Unavailable,
}

impl TextInputSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::InputMethodV2 => "input-method-v2 (read/write)",
            Self::VirtualKeyboard => "virtual-keyboard (write-only)",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Protocol capabilities discovered during startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextInputCapabilities {
    pub input_method_v2: bool,
    pub text_input_v3: bool,
    pub virtual_keyboard_v1: bool,
}

impl TextInputCapabilities {
    pub const fn preferred_source(self) -> TextInputSource {
        if self.input_method_v2 {
            TextInputSource::InputMethodV2
        } else if self.virtual_keyboard_v1 {
            TextInputSource::VirtualKeyboard
        } else {
            TextInputSource::Unavailable
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct InputMethodState {
    pub is_active: Arc<AtomicBool>,
    pub serial: u32,
    pub surrounding_text: Option<String>,
    pub cursor_pos: u32,
    pub anchor_pos: u32,
    pub content_hint: u32,
    pub content_purpose: u32,
    pub last_content_purpose: Option<u32>,
    pub source: TextInputSource,
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

// This manager is probed for diagnostics only. text-input-v3 is an
// app-to-compositor protocol; binding its manager does not grant a client
// permission to inspect another app's text.
impl Dispatch<ZwpTextInputManagerV3, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwpTextInputManagerV3,
        _event: <ZwpTextInputManagerV3 as wayland_client::Proxy>::Event,
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
                state.im_state.anchor_pos = 0;
                state.im_state.source = TextInputSource::InputMethodV2;
                state.suggest_engine.clear();
                if state.config.behavior.auto_show && state.allow_auto_show() {
                    state.show_keyboard(qh);
                    // If triggered by auto-show, allow auto-hide on blur
                    state.manually_shown = false;
                }
            }
            zwp_input_method_v2::Event::Deactivate => {
                tracing::info!(
                    "Received text-input Deactivate event (manually_shown={})",
                    state.manually_shown
                );
                state.im_state.is_active.store(false, Ordering::SeqCst);
                state.im_state.last_content_purpose = None;
                state.im_state.anchor_pos = 0;
                state.im_state.source = TextInputSource::VirtualKeyboard;
                state.suggest_engine.clear();
                if state.config.behavior.auto_hide && !state.manually_shown {
                    state.hide_keyboard(qh);
                }
            }
            zwp_input_method_v2::Event::SurroundingText {
                text,
                cursor,
                anchor,
            } => {
                let has_selection = cursor != anchor;
                // For an inline completion, the host reports the typed prefix
                // and the predicted suffix as a selection. The lower endpoint
                // is the actual typed-prefix cursor used by the suggester.
                let sync_cursor = cursor.min(anchor);

                // Chromium can deliver delayed, non-selection echoes while a
                // word is being typed. Do not let those overwrite either the
                // local word or its replacement cache. A real selection is
                // authoritative for replacement, even while current_word is
                // non-empty, so retain both endpoints in that case.
                let stale_echo = !has_selection
                    && !text.is_empty()
                    && !state.suggest_engine.current_word.is_empty();
                if !stale_echo {
                    state.im_state.surrounding_text = Some(text.clone());
                    state.im_state.cursor_pos = cursor;
                    state.im_state.anchor_pos = anchor;
                    if text.is_empty() || state.suggest_engine.current_word.is_empty() {
                        state.sync_suggest_with_surrounding(&text, sync_cursor, qh);
                    }
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
                tracing::warn!(
                    "zwp_input_method_v2 became unavailable; another input method may already own this seat"
                );
                state.im_state.is_active.store(false, Ordering::SeqCst);
                state.im_state.source = TextInputSource::VirtualKeyboard;
                state.input_method = None;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TextInputCapabilities, TextInputSource};

    #[test]
    fn capabilities_prefer_readable_input_method() {
        let capabilities = TextInputCapabilities {
            input_method_v2: true,
            text_input_v3: true,
            virtual_keyboard_v1: true,
        };
        assert_eq!(
            capabilities.preferred_source(),
            TextInputSource::InputMethodV2
        );
    }

    #[test]
    fn capabilities_fall_back_to_virtual_keyboard() {
        let capabilities = TextInputCapabilities {
            input_method_v2: false,
            text_input_v3: true,
            virtual_keyboard_v1: true,
        };
        assert_eq!(
            capabilities.preferred_source(),
            TextInputSource::VirtualKeyboard
        );
    }

    #[test]
    fn capabilities_report_unavailable_without_protocols() {
        assert_eq!(
            TextInputCapabilities::default().preferred_source(),
            TextInputSource::Unavailable
        );
    }
}
