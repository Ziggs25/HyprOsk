use crate::layout::key::KeyAction;
use crate::wayland::state::WaylandState;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3;
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::ChangeCause;
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
        self.label_with_readback(true)
    }

    pub const fn label_with_readback(self, readback_enabled: bool) -> &'static str {
        match self {
            Self::InputMethodV2 => {
                if readback_enabled {
                    "input-method-v2 (read/write)"
                } else {
                    "input-method-v2 (write-only, readback disabled)"
                }
            }
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
    pub pending_surrounding_text: Option<(String, u32, u32)>,
    pub pending_external_change: bool,
    pub pending_arrow_navigation: bool,
    pub last_wayland_snapshot_at: Option<Instant>,
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
                let readback = state.config.behavior.surrounding_text_readback;
                tracing::info!(
                    "Received text-input Activate event (readback={})",
                    if readback { "enabled" } else { "disabled" }
                );
                state.im_state.is_active.store(true, Ordering::SeqCst);
                state.im_state.last_content_purpose = None;
                state.im_state.surrounding_text = None;
                state.im_state.cursor_pos = 0;
                state.im_state.anchor_pos = 0;
                state.im_state.pending_surrounding_text = None;
                state.im_state.pending_external_change = false;
                state.im_state.pending_arrow_navigation = false;
                state.im_state.last_wayland_snapshot_at = None;
                state.im_state.source = TextInputSource::InputMethodV2;
                state.suggest_engine.clear();
                if state.config.behavior.auto_show {
                    if state.allow_auto_show() {
                        state.show_keyboard_with_auto_placement(qh);
                        // If triggered by auto-show, allow auto-hide on blur
                        state.manually_shown = false;
                    } else {
                        tracing::info!(
                            "Auto-show suppressed by policy (folio_mode={}, folio_attached={}, touch_only={})",
                            state.config.behavior.folio_mode,
                            state.folio_attached,
                            state.config.behavior.touch_only
                        );
                    }
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
                state.im_state.pending_surrounding_text = None;
                state.im_state.pending_external_change = false;
                state.im_state.pending_arrow_navigation = false;
                state.im_state.last_wayland_snapshot_at = None;

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
                if !state.config.behavior.surrounding_text_readback {
                    state.im_state.surrounding_text = None;
                    state.im_state.cursor_pos = 0;
                    state.im_state.anchor_pos = 0;
                    state.im_state.pending_surrounding_text = None;
                    return;
                }
                if matches!(
                    state.repeat_key.as_ref(),
                    Some(KeyAction::ArrowUp | KeyAction::ArrowDown)
                ) || state
                    .last_arrow_at
                    .map_or(false, |time| time.elapsed() < std::time::Duration::from_millis(500))
                {
                    state.im_state.pending_arrow_navigation = true;
                }
                tracing::debug!(
                    surrounding_bytes = text.len(),
                    cursor,
                    anchor,
                    "received host surrounding-text snapshot"
                );
                state.im_state.pending_surrounding_text = Some((text, cursor, anchor));
            }
            zwp_input_method_v2::Event::TextChangeCause { cause } => {
                let input_method_change = matches!(cause, WEnum::Value(ChangeCause::InputMethod));
                state.im_state.pending_external_change = !input_method_change;
                tracing::debug!(input_method_change, "received host text-change cause");
            }
            zwp_input_method_v2::Event::ContentType { hint, purpose } => {
                let p: u32 = purpose.into();
                let h: u32 = hint.into();
                state.im_state.content_hint = h;
                state.im_state.content_purpose = p;
                state.adapt_layout_for_content_purpose(p, qh);
            }
            zwp_input_method_v2::Event::Done => {
                state.im_state.serial = state.im_state.serial.wrapping_add(1);
                let external = std::mem::take(&mut state.im_state.pending_external_change);
                let arrow_navigation = std::mem::take(&mut state.im_state.pending_arrow_navigation);
                if let Some((text, cursor, anchor)) = state.im_state.pending_surrounding_text.take() {
                    let valid = (cursor as usize) <= text.len()
                        && (anchor as usize) <= text.len()
                        && text.is_char_boundary(cursor as usize)
                        && text.is_char_boundary(anchor as usize);
                    if !valid {
                        tracing::warn!("ignoring surrounding text with invalid cursor or anchor");
                        return;
                    }

                    if arrow_navigation {
                        state.im_state.last_wayland_snapshot_at = Some(Instant::now());
                        state.im_state.surrounding_text = Some(text);
                        state.im_state.cursor_pos = cursor;
                        state.im_state.anchor_pos = anchor;
                        state.suggest_engine.clear();
                        state.sync_layout_and_redraw(qh);
                        return;
                    }

                    if !external
                        && state.suggest_engine.is_recent_inline_completion_echo(
                            &text,
                            cursor as usize,
                            anchor as usize,
                        )
                    {
                        tracing::info!(
                            surrounding_bytes = text.len(),
                            cursor,
                            anchor,
                            "ignoring stale inline-completion selection echo"
                        );
                        return;
                    }

                    let moved = cursor != state.im_state.cursor_pos
                        || anchor != state.im_state.anchor_pos;
                    // A normal input-method echo may trail local typing by a character.
                    // An unchanged host buffer with a new caret, or a caret moving
                    // backwards, is navigation and must rebuild the word context.
                    let caret_navigation = moved
                        && (state.im_state.surrounding_text.as_deref() == Some(text.as_str())
                            || cursor < state.im_state.cursor_pos);
                    let authoritative = external || cursor != anchor || caret_navigation;
                    let before = &text[..cursor as usize];
                    let echoed_word = before
                        .rsplit(|ch: char| !ch.is_alphabetic() && ch != '\'')
                        .next()
                        .unwrap_or("");
                    let tracked_word = &state.suggest_engine.current_word;
                    let stale_prefix = !tracked_word.is_empty()
                        && (echoed_word.is_empty()
                            || (echoed_word.len() < tracked_word.len()
                                && tracked_word.starts_with(echoed_word)));
                    if !authoritative && stale_prefix {
                        tracing::debug!(cursor, anchor, "ignoring delayed input-method echo");
                        return;
                    }
                    state.im_state.last_wayland_snapshot_at = Some(Instant::now());

                    tracing::info!(
                        surrounding_bytes = text.len(),
                        cursor,
                        anchor,
                        external,
                        moved,
                        authoritative,
                        "applying host surrounding-text update"
                    );
                    state.im_state.surrounding_text = Some(text.clone());
                    state.im_state.cursor_pos = cursor;
                    state.im_state.anchor_pos = anchor;
                    state.sync_suggest_with_surrounding(
                        &text,
                        cursor.min(anchor),
                        authoritative,
                        qh,
                    );
                } else if external {
                    tracing::debug!(
                        "host reported an external text change without surrounding text"
                    );
                }
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
