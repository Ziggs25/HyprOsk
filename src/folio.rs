//! Physical keyboard (folio) detection.
//!
//! Hyprosk normally auto-shows on text-input focus. In folio mode
//! ([`crate::config::BehaviorConfig::folio_mode`]) it only does so while no
//! usable physical keyboard is present.
//!
//! Two sources of truth, both read directly from the kernel:
//!
//! - **`SW_TABLET_MODE`**: on 2-in-1/convertible machines the kernel flips
//!   this switch when the detachable keyboard folio is detached. This is the
//!   authoritative "no keyboard" signal because on many machines (e.g.
//!   i8042/AT based folios) the input device *stays registered* after detach,
//!   so device-list scanning alone can never detect removal.
//! - **evdev letter-key bitmap**: a device that can actually type letters
//!   (`KEY_A..KEY_Z`, `KEY_1..KEY_0` via `EVIOCGBIT`) counts as a physical
//!   keyboard. This filters out spurious `kbd`-handler devices (hotkey
//!   arrays, brightness/volume buttons, sleep buttons, WMI events).
//!
//! If `/dev/input/event*` is not readable (not in the `input` group), a
//! fallback scans `/proc/bus/input/devices` with the same letter-key logic
//! applied to the `B: KEY` bitmaps.
//!
//! The zwp_virtual_keyboard_v1 interface we use does not create an evdev
//! device, so the on-screen keyboard cannot detect itself.

use std::fs;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use libc;

// EVIOCGBIT(ev, len) = _IOC(_IOC_READ, 'E', 0x20 + ev, len)
const _IOC_READ: libc::c_ulong = 2;
const INPUT_TYPE: libc::c_ulong = b'E' as libc::c_ulong;

fn eviocgbit(ev: u32, len: usize) -> libc::c_ulong {
    (_IOC_READ << 30) | ((len as libc::c_ulong) << 16) | (INPUT_TYPE << 8) | (0x20 + ev) as libc::c_ulong
}

/// EVIOCGSW = _IOC(_IOC_READ, 'E', 0x1b, 8)
fn eviocgsw() -> libc::c_ulong {
    (_IOC_READ << 30) | (8 << 16) | (INPUT_TYPE << 8) | 0x1b
}

const EV_KEY: u32 = 0x01;
const EV_SW: u32 = 0x05;
const KEY_1: usize = 2;
const KEY_0: usize = 11;
const KEY_A: usize = 30;
const KEY_Z: usize = 44;
const SW_TABLET_MODE: usize = 0x01;

/// Bitmap size for the key capability: (KEY_MAX + 7) / 8 = 96 bytes.
const KEY_BITMAP_BYTES: usize = 96;
const SW_BITMAP_BYTES: usize = 8;

/// True when any evdev device reports `SW_TABLET_MODE` set (folio detached).
pub fn tablet_mode_active() -> bool {
    let Ok(entries) = fs::read_dir("/dev/input") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(stem) = name.to_str().filter(|s| s.starts_with("event")) else {
            continue;
        };
        let path = format!("/dev/input/{stem}");
        let fd = unsafe { libc::open(std::ffi::CString::new(path).unwrap().as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        if fd < 0 {
            continue;
        }

        // Verify device supports EV_SW and specifically SW_TABLET_MODE
        let mut sw_cap = [0u8; SW_BITMAP_BYTES];
        let cap_ret = unsafe { libc::ioctl(fd, eviocgbit(EV_SW, SW_BITMAP_BYTES), sw_cap.as_mut_ptr()) };
        if cap_ret < 0 || (sw_cap[SW_TABLET_MODE / 8] >> (SW_TABLET_MODE % 8)) & 1 == 0 {
            unsafe { libc::close(fd) };
            continue;
        }

        let mut sw: [u8; 8] = [0; 8];
        let ret = unsafe { libc::ioctl(fd, eviocgsw(), sw.as_mut_ptr()) };
        unsafe { libc::close(fd) };
        if ret < 0 {
            continue;
        }
        if (sw[SW_TABLET_MODE / 8] >> (SW_TABLET_MODE % 8)) & 1 != 0 {
            return true;
        }
    }
    false
}

pub fn key_bitmap(fd: libc::c_int) -> Option<[u8; KEY_BITMAP_BYTES]> {
    let mut bits = [0u8; KEY_BITMAP_BYTES];
    let ret = unsafe { libc::ioctl(fd, eviocgbit(EV_KEY, KEY_BITMAP_BYTES), bits.as_mut_ptr()) };
    if ret < 0 {
        return None;
    }
    Some(bits)
}

fn has_letter_keys(bits: &[u8]) -> bool {
    (KEY_1..=KEY_0).chain(KEY_A..=KEY_Z).any(|i| (bits[i / 8] >> (i % 8)) & 1 != 0)
}

/// True when any readable evdev device has typeable letter keys.
pub fn evdev_keyboard_present() -> bool {
    let Ok(entries) = fs::read_dir("/dev/input") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(stem) = name.to_str().filter(|s| s.starts_with("event")) else {
            continue;
        };
        let path = format!("/dev/input/{stem}");
        let fd = unsafe { libc::open(std::ffi::CString::new(path).unwrap().as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        if fd < 0 {
            continue;
        }
        let has_letters = key_bitmap(fd).is_some_and(|b| has_letter_keys(&b));
        unsafe { libc::close(fd) };
        if has_letters {
            return true;
        }
    }
    false
}

/// Fallback: parse `/proc/bus/input/devices` and apply the same letter-key
/// test to the `B: KEY` bitmap of each `kbd` device block. The bitmap is
/// printed as space-separated 8-byte hex words, most-significant byte of
/// each word first (i.e. the word is byte-reversed vs. memory order).
pub fn proc_keyboard_present() -> bool {
    let Ok(devices) = fs::read_to_string("/proc/bus/input/devices") else {
        return false;
    };
    for block in devices.split("\n\n") {
        let has_kbd = block.lines().any(|l| l.starts_with("H:") && l.split_whitespace().any(|h| h == "kbd"));
        if !has_kbd {
            continue;
        }
        let Some(key_line) = block.lines().find(|l| l.starts_with("B: KEY=")) else {
            continue;
        };
        let words = key_line.trim_start_matches("B: KEY=").split_whitespace();
        let mut bits = [0u8; KEY_BITMAP_BYTES];
        let mut consumed = 0;
        for word in words {
            if consumed >= bits.len() {
                break;
            }
            let Ok(w) = u64::from_str_radix(word, 16) else {
                continue;
            };
            let bytes = w.to_le_bytes();
            for k in 0..8 {
                if consumed + k < bits.len() {
                    bits[consumed + k] = bytes[7 - k];
                }
            }
            consumed += 8;
        }
        if has_letter_keys(&bits) {
            return true;
        }
    }
    false
}

/// Check if the physical folio USB device (Alps Folio Keyboard / Touchpad 044e:1218)
/// is physically attached to the USB bus.
pub fn usb_folio_present() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") {
        for entry in entries.flatten() {
            let path = entry.path();
            let vendor_path = path.join("idVendor");
            let product_path = path.join("idProduct");
            if let Ok(vendor) = fs::read_to_string(&vendor_path)
                && let Ok(product) = fs::read_to_string(&product_path)
            {
                if vendor.trim().eq_ignore_ascii_case("044e") && product.trim().eq_ignore_ascii_case("1218") {
                    return true;
                }
            }
        }
    }
    false
}

/// Returns true when at least one usable physical keyboard is present
/// (i.e. the folio is attached).
pub fn physical_keyboard_attached() -> bool {
    // 1. Direct hardware USB presence of the Dell Latitude Folio:
    // If the folio is physically plugged into the pogo pins, its USB hub is powered and present.
    if usb_folio_present() {
        // If the folio is physically attached, only consider it detached if tablet mode is explicitly active (folded back 360 deg)
        if tablet_mode_active() {
            tracing::debug!("SW_TABLET_MODE active (folded back), folio considered detached");
            return false;
        }
        return true;
    }

    // 2. Fallback for non-USB folios or external keyboards:
    if tablet_mode_active() {
        tracing::debug!("SW_TABLET_MODE active, folio considered detached");
        return false;
    }
    if evdev_keyboard_present() {
        return true;
    }
    let fallback = proc_keyboard_present();
    if fallback {
        tracing::debug!("No evdev access, /proc fallback found a keyboard");
    }
    fallback
}

/// Spawns a background thread that polls folio presence with debouncing
/// and sends each confirmed change through `tx` (`true` = attached, `false` = detached).
pub fn spawn_folio_poller(tx: Sender<bool>) {
    thread::spawn(move || {
        let mut last = physical_keyboard_attached();
        loop {
            thread::sleep(Duration::from_millis(500));
            let now = physical_keyboard_attached();
            if now != last {
                // Debounce: verify the state again after 300ms to eliminate mechanical jitter
                thread::sleep(Duration::from_millis(300));
                let confirmed = physical_keyboard_attached();
                if confirmed == now {
                    let _ = tx.send(confirmed);
                    last = confirmed;
                }
            }
        }
    });
}