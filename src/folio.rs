//! Physical keyboard (folio) detection.
//!
//! Hyprosk normally auto-shows on text-input focus. In folio mode
//! ([`crate::config::BehaviorConfig::folio_mode`]) it only does so while no
//! usable physical keyboard is present.
//!
//! Detection reads four kernel sources, most specific first:
//!
//! - **USB identity `044e:1218` (Alps Touchpad)**: the Latitudes with a
//!   pogo-pin folio expose the keyboard *and* its trackpad as a single USB
//!   HID touchpad. `044e:1218` present therefore means the folio is docked.
//!   On this hardware the device stays powered through 360-degree folding,
//!   so its presence means "attached", never "usable in laptop posture".
//! - **`SW_TABLET_MODE`**: posture source. Read *only* to reject a
//!   `044e:1218` result, because the value cannot be trusted on its own:
//!   `intel-hid` mostly ships it `TABLET_SW_AT_EVENT` (only Dell rugged/
//!   Plus 2-in-1 SKUs are on the VGBS allow list), so it is uninitialized
//!   until the first `0xcc`/`0xcd` ACPI notify, and it latches if the
//!   firmware never resends that notify across s2idle/hibernate.
//! - **evdev letter-key bitmap**: a device that can actually type letters
//!   (`KEY_A..KEY_Z`, `KEY_1..KEY_0` via `EVIOCGBIT`) counts as a physical
//!   keyboard. This filters out spurious `kbd`-handler devices (hotkey
//!   arrays, brightness/volume buttons, sleep buttons, WMI events).
//! - **`/proc/bus/input/devices`**: same letter-key logic on the `B: KEY`
//!   bitmaps, for when `/dev/input/event*` is unreadable (not in `input`).
//!
//! Ordering matters: `044e:1218` and the letter-key scan are *observed*
//! state, so they win; `SW_TABLET_MODE` is a *posture* hint that stays
//! authoritative only over the bare-US-keyboard case. If the fold state can
//! never be observed (see tests for the 360-degree caveat), attachment wins:
//! suppressing the OSK is a better failure than covering a physical keyboard.
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

/// USB vendor/product of the Dell Latitude folio: the keyboard and its
/// trackpad hang off one Alps USB HID device.
pub const FOLIO_USB_VENDOR: &str = "044e";
pub const FOLIO_USB_PRODUCT: &str = "1218";

/// Which kernel source produced a verdict, so the daemon log and `hyprosk
/// status` can explain *why* auto-show was suppressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolioSource {
    /// `044e:1218` is on the USB bus.
    UsbFolio,
    /// A letter-capable evdev device is readable.
    EvdevKeyboard,
    /// A letter-capable `B: KEY` bitmap in `/proc/bus/input/devices`.
    ProcKeyboard,
    /// Only `SW_TABLET_MODE` matched; no physical keyboard was observed.
    PostureSwitch,
    /// Nothing matched.
    None,
}

impl FolioSource {
    /// True when the source proves usable typing hardware instead of merely
    /// describing posture.
    pub fn is_keyboard(self) -> bool {
        matches!(self, Self::UsbFolio | Self::EvdevKeyboard | Self::ProcKeyboard)
    }

    /// Short stable name for logs and `hyprosk status`.
    pub fn label(self) -> &'static str {
        match self {
            Self::UsbFolio => "usb-044e:1218",
            Self::EvdevKeyboard => "evdev-letter-keys",
            Self::ProcKeyboard => "proc-letter-keys",
            Self::PostureSwitch => "sw-tablet-mode",
            Self::None => "none",
        }
    }
}

/// One-shot snapshot of every detection source, so callers never mix reads
/// from different points in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolioStatus {
    /// `SW_TABLET_MODE` is set on some evdev switch device.
    pub tablet_mode: bool,
    /// `044e:1218` is enumerated on the USB bus.
    pub usb_folio: bool,
    /// A readable evdev device can type letters.
    pub evdev_keyboard: bool,
    /// `/proc/bus/input/devices` has a letter-capable keyboard.
    pub proc_keyboard: bool,
    /// Verdict: a physical keyboard is attached, so auto-show stays suppressed.
    pub attached: bool,
    /// Source that produced `attached`.
    pub source: FolioSource,
}

impl FolioStatus {
    /// Fresh snapshot of the kernel state.
    pub fn detect() -> Self {
        let tablet_mode = tablet_mode_active();
        let usb_folio = usb_folio_present();
        let evdev_keyboard = evdev_keyboard_present();
        let proc_keyboard = evdev_keyboard || proc_keyboard_present();
        let (attached, source) = verdict(tablet_mode, usb_folio, evdev_keyboard, proc_keyboard);
        Self {
            tablet_mode,
            usb_folio,
            evdev_keyboard,
            proc_keyboard,
            attached,
            source,
        }
    }
}

/// Compact value for change-detection by the poller. Posture transitions are
/// ignored because they are not trustworthy on their own and would leak the
/// flaky `SW_TABLET_MODE` value into the debounce logic.
fn attach_signature(status: &FolioStatus) -> (bool, FolioSource) {
    (status.attached, status.source)
}

/// Decide and explain the attachment verdict from already-sampled sources.
fn verdict(
    tablet_mode: bool,
    usb_folio: bool,
    evdev_kbd: bool,
    proc_kbd: bool,
) -> (bool, FolioSource) {
    if usb_folio {
        // Attached in either posture: 360-degree folding leaves 044e:1218
        // powered, and `tablet_mode` latches after s2idle. Prefer the
        // directly observed USB device so a fold cannot re-enable auto-show.
        let source = FolioSource::UsbFolio;
        if tablet_mode {
            tracing::debug!(
                "folio USB 044e:1218 present while SW_TABLET_MODE is set; treating folio as attached"
            );
        } else {
            tracing::debug!("physical folio detected by USB ID 044e:1218");
        }
        return (true, source);
    }

    // An observed letter-capable keyboard is real typing hardware. Surface
    // keyboards and i8042 keyboards must suppress the OSK in folio mode even
    // when the posture switch is stuck on after resume.
    if evdev_kbd {
        if tablet_mode {
            tracing::debug!(
                "letter-capable evdev keyboard present while SW_TABLET_MODE is set; trusting the keyboard"
            );
        }
        return (true, FolioSource::EvdevKeyboard);
    }
    if proc_kbd {
        tracing::debug!("no evdev access, /proc fallback found a letter-capable keyboard");
        return (true, FolioSource::ProcKeyboard);
    }

    // No keyboard was observed. Trust the posture switch only here: with no
    // keyboard to type on, suppressing the OSK would leave the user stranded.
    if tablet_mode {
        tracing::debug!("SW_TABLET_MODE active with no physical keyboard; folio considered detached");
        return (false, FolioSource::PostureSwitch);
    }

    (false, FolioSource::None)
}

/// True when any evdev device currently reports `SW_TABLET_MODE` set.
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
            tracing::debug!("SW_TABLET_MODE active on {stem}");
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

/// Extract `(bus, port)` from a USB sysfs device directory name.
///
/// `/sys/bus/usb/devices` is a *flat* list of every USB entity: a name with a
/// colon is an interface (`1-4:1.0`) and has no `idVendor` of its own, while
/// `usb1` is a root hub. Skipping both keeps the scan to device paths.
fn usb_devpath_ids(name: &str) -> Option<(&str, &str)> {
    let (bus, rest) = name.split_once('-')?;
    let port = rest.split('.').next()?;
    if bus.is_empty()
        || port.is_empty()
        || !bus.bytes().all(|b| b.is_ascii_digit())
        || !rest.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b':')
        || rest.contains(':')
    {
        return None;
    }
    Some((bus, port))
}

/// True when a `044e:1218` device is enumerated on the USB bus.
///
/// Entries are matched by sysfs directory name instead of the `usb1`/`usb2`
/// root-hub symlinks, so the scan stays bounded and a device that disappears
/// between `read_dir` and `read_to_string` cannot abort the whole check.
/// `idVendor` and `idProduct` are read independently: after s2idle the folio
/// can re-enumerate and briefly expose only one of the two attributes.
pub fn usb_folio_present() -> bool {
    let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return false;
        };
        if usb_devpath_ids(name).is_none() {
            return false;
        }
        let vendor = fs::read_to_string(entry.path().join("idVendor")).ok();
        let product = fs::read_to_string(entry.path().join("idProduct")).ok();
        match (vendor, product) {
            (Some(vendor), Some(product)) => {
                vendor.trim().eq_ignore_ascii_case(FOLIO_USB_VENDOR)
                    && product.trim().eq_ignore_ascii_case(FOLIO_USB_PRODUCT)
            }
            _ => false,
        }
    })
}

/// Returns true when the known folio USB device or a letter-capable keyboard is present.
pub fn physical_keyboard_attached() -> bool {
    FolioStatus::detect().attached
}

/// Spawns a background thread that polls folio presence with debouncing
/// and sends each confirmed change through `tx` (`true` = attached, `false` = detached).
pub fn spawn_folio_poller(tx: Sender<bool>) {
    thread::spawn(move || {
        let mut last = physical_keyboard_attached();
        loop {
            thread::sleep(Duration::from_millis(500));
            let now = FolioStatus::detect();
            let signature = attach_signature(&now);
            if now.attached != last {
                // Debounce: verify the state again after 300ms to eliminate mechanical jitter
                thread::sleep(Duration::from_millis(300));
                let confirmed = FolioStatus::detect();
                if attach_signature(&confirmed) == signature {
                    let _ = tx.send(confirmed.attached);
                    last = confirmed.attached;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real-world attach/posture matrix (Dell Latitude 7200 2-in-1, BIOS 1.39,
    // Intel HID switches on INT33D5:00, Alps Touchpad on usb1 port 4):
    //
    // | folio    | posture switch    | 044e:1218 | letter keys | result   |
    // |----------|-------------------|-----------|-------------|----------|
    // | docked   | inactive          | yes       | no          | attached |
    // | docked   | stuck on (resume) | yes       | no          | attached |
    // | folded   | active            | yes       | no          | attached |
    // | folded   | stuck inactive    | yes       | no          | attached |
    // | detached | active            | no        | no          | detached |
    //
    // 044e:1218 is a touchpad-only HID device, so it can never satisfy the
    // letter-key scan; the folio keyboard is multiplexed over the same USB
    // device and is not a separate evdev keyboard.

    #[test]
    fn usb_folio_wins_over_an_inactive_posture_switch() {
        // 360-degree fold where the firmware never sent 0xcc: the USB ID is the
        // only evidence left and it looks exactly like laptop posture. Prefer
        // attached so folding the folio cannot re-enable auto-show.
        let (attached, source) = verdict(false, true, false, false);
        assert!(attached);
        assert_eq!(source, FolioSource::UsbFolio);
    }

    #[test]
    fn usb_folio_wins_over_a_stuck_active_posture_switch() {
        // Laptop posture after resume: the alps device never resends
        // SW_TABLET_MODE=0, so the switch stays latched on.
        let (attached, source) = verdict(true, true, false, false);
        assert!(attached);
        assert_eq!(source, FolioSource::UsbFolio);
    }

    #[test]
    fn detached_folio_falls_back_to_the_posture_switch() {
        assert_eq!(
            verdict(true, false, false, false),
            (false, FolioSource::PostureSwitch)
        );
    }

    #[test]
    fn detached_folio_with_stable_posture_reports_none() {
        assert_eq!(verdict(false, false, false, false), (false, FolioSource::None));
    }

    #[test]
    fn separately_attached_keyboard_survives_a_stuck_posture_switch() {
        // External USB or i8042 keyboard: suppress auto-show even if the switch
        // is still set after wake, because the user can demonstrably type.
        assert_eq!(
            verdict(true, false, true, false),
            (true, FolioSource::EvdevKeyboard)
        );
    }

    #[test]
    fn proc_fallback_counts_when_evdev_is_unreadable() {
        assert_eq!(
            verdict(false, false, false, true),
            (true, FolioSource::ProcKeyboard)
        );
    }

    #[test]
    fn posture_switch_alone_never_gates_auto_show() {
        assert!(!FolioSource::PostureSwitch.is_keyboard());
        assert!(!FolioSource::None.is_keyboard());
        assert!(FolioSource::UsbFolio.is_keyboard());
        assert!(FolioSource::EvdevKeyboard.is_keyboard());
        assert!(FolioSource::ProcKeyboard.is_keyboard());
    }

    #[test]
    fn poller_signature_ignores_posture_only_changes() {
        let base = FolioStatus {
            tablet_mode: false,
            usb_folio: true,
            evdev_keyboard: false,
            proc_keyboard: false,
            attached: true,
            source: FolioSource::UsbFolio,
        };
        let flipped = FolioStatus {
            tablet_mode: true,
            ..base
        };
        // A flaky switch flip must not be debounced into an attach/detach event.
        assert_eq!(attach_signature(&base), attach_signature(&flipped));
    }

    #[test]
    fn usb_devpath_ids_rejects_interfaces_and_root_hubs() {
        assert_eq!(usb_devpath_ids("1-4"), Some(("1", "4")));
        assert_eq!(usb_devpath_ids("2-1.3"), Some(("2", "1")));
        assert_eq!(usb_devpath_ids("1-4:1.0"), None);
        assert_eq!(usb_devpath_ids("1-0:1.0"), None);
        assert_eq!(usb_devpath_ids("usb1"), None);
        assert_eq!(usb_devpath_ids("1-"), None);
    }
}
