use std::ffi::CString;
use std::os::fd::OwnedFd;

use nix::sys::memfd::{memfd_create, MemFdCreateFlag};
use nix::unistd::write;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::{self, ZwpVirtualKeyboardV1},
};
use xkbcommon::xkb::{Context, Keymap, CONTEXT_NO_FLAGS, KEYMAP_COMPILE_NO_FLAGS, KEYMAP_FORMAT_TEXT_V1};

use crate::wayland::state::WaylandState;

/// Protocol identifier for xkb text-v1 keymaps (XKB_KEYMAP_FORMAT_TEXT_V1).
pub const KEYMAP_FORMAT: u32 = KEYMAP_FORMAT_TEXT_V1;

/// Compiles a default US keymap and returns an `(fd, size)` pair ready for a
/// `keymap` request on either the virtual keyboard or input method proxy.
///
/// The returned fd is owned by the caller and closed automatically once sent.
pub fn create_keymap_fd() -> Option<(OwnedFd, u32)> {
    let context = Context::new(CONTEXT_NO_FLAGS);
    let keymap = Keymap::new_from_names(
        &context,
        "",
        "",
        "us",
        "",
        None,
        KEYMAP_COMPILE_NO_FLAGS,
    )?;
    let text = keymap.get_as_string(KEYMAP_FORMAT_TEXT_V1);

    let name = CString::new("hyprosk-keymap").ok()?;
    let fd = memfd_create(&name, MemFdCreateFlag::empty()).ok()?;
    write(&fd, text.as_bytes()).ok()?;

    Some((fd, text.len() as u32))
}

impl Dispatch<ZwpVirtualKeyboardManagerV1, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwpVirtualKeyboardManagerV1,
        _event: <ZwpVirtualKeyboardManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpVirtualKeyboardV1, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwpVirtualKeyboardV1,
        _event: zwp_virtual_keyboard_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}