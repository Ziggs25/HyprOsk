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
//!   (`KEY_A..KEY_Z`, `KEY_1..KEY_0` via `EVIOCGBIT`) counts as a keyboard.
//!   Crucially, only *external* devices count: the built-in `i8042` AT
//!   keyboard and the ACPI/WMI hotkey arrays keep their letter bits forever on
//!   a 2-in-1, even with the folio detached, so they are classified out by
//!   kernel device path (`/devices/platform/i8042/...`,
//!   `/devices/platform/INT33D5:00/...`). Otherwise every 2-in-1 would look
//!   like it had a keyboard attached permanently.
//! - **`/proc/bus/input/devices`**: same letter-key logic on the `B: KEY`
//!   bitmaps, used when `/dev/input/event*` is unreadable (not in `input`).
//!
//! Ordering matters: `044e:1218` and an *external* keyboard are *observed*
//! state, so they win; `SW_TABLET_MODE` is a *posture* hint that stays
//! authoritative whenever no external keyboard is present. A missing built-in
//! keyboard (uninitialized `i8042` after resume) only corroborates the switch.
//! If the fold state can never be observed (see tests for the 360-degree
//! caveat), attachment wins: suppressing the OSK is a better failure than
//! covering a physical keyboard.
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
    (_IOC_READ << 30)
        | ((len as libc::c_ulong) << 16)
        | (INPUT_TYPE << 8)
        | (0x20 + ev) as libc::c_ulong
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
        matches!(
            self,
            Self::UsbFolio | Self::EvdevKeyboard | Self::ProcKeyboard
        )
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
    /// An external (hot-pluggable) letter-capable keyboard is present.
    pub external_keyboard: bool,
    /// The built-in `i8042` / ACPI / WMI keyboard devices expose letter keys.
    /// Informational only: this stays `true` on a 2-in-1 even when the folio is
    /// detached, so it never counts as an attachment on its own.
    pub builtin_keyboard: bool,
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
        let external_keyboard = external_keyboard_present();
        let builtin_keyboard = builtin_keyboard_present();
        let (attached, source) =
            verdict(tablet_mode, usb_folio, external_keyboard, builtin_keyboard);
        Self {
            tablet_mode,
            usb_folio,
            external_keyboard,
            builtin_keyboard,
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
///
/// Precedence, most specific first:
///
/// 1. `044e:1218` on the USB bus -> the folio is physically docked.
/// 2. An *external* letter-capable keyboard -> something the user can type on.
/// 3. No keyboard observed: the posture switch is the only remaining signal, so
///    an active `SW_TABLET_MODE` means the folio is away.
///
/// The built-in `i8042` AT keyboard and the ACPI/WMI hotkey arrays are
/// deliberately excluded from step 2: they keep their letter-key bits forever,
/// so they must never be read as "a keyboard is attached". Their letter bits
/// only *add* evidence to step 3: if even the built-in keyboard is missing (an
/// uninitialized `i8042` after resume), an active switch is additionally
/// corroborated instead of being trusted alone.
fn verdict(
    tablet_mode: bool,
    usb_folio: bool,
    external_keyboard: bool,
    builtin_keyboard: bool,
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

    // External keyboard means a device that disappears on unplug. The built-in
    // i8042 and ACPI/WMI hotkey arrays are classified out in
    // `external_keyboard_present()`, because on this 2-in-1 they never vanish.
    if external_keyboard {
        if tablet_mode {
            tracing::debug!(
                "external letter-capable keyboard present while SW_TABLET_MODE is set; trusting the keyboard"
            );
        }
        return (true, FolioSource::EvdevKeyboard);
    }

    // No keyboard was observed. The posture switch is now the only signal, and
    // with no keyboard to type on, suppressing the OSK would strand the user.
    if tablet_mode {
        let corroborated = if builtin_keyboard {
            "the built-in keyboard is present"
        } else {
            "even the built-in keyboard is unavailable"
        };
        tracing::debug!(
            "SW_TABLET_MODE active, no external keyboard, {corroborated}; folio considered detached"
        );
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
        let fd = unsafe {
            libc::open(
                std::ffi::CString::new(path).unwrap().as_ptr(),
                libc::O_RDONLY | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            continue;
        }

        // Verify device supports EV_SW and specifically SW_TABLET_MODE
        let mut sw_cap = [0u8; SW_BITMAP_BYTES];
        let cap_ret =
            unsafe { libc::ioctl(fd, eviocgbit(EV_SW, SW_BITMAP_BYTES), sw_cap.as_mut_ptr()) };
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
    (KEY_1..=KEY_0)
        .chain(KEY_A..=KEY_Z)
        .any(|i| (bits[i / 8] >> (i % 8)) & 1 != 0)
}

/// True when any readable evdev device has typeable letter keys.
/// Letter-capable keyboards readable through evdev, with their kernel paths
/// resolved from `/proc/bus/input/devices` so they can be classified.
pub fn evdev_letter_keyboards() -> Vec<KeyboardIdentity> {
    let Ok(entries) = fs::read_dir("/dev/input") else {
        return Vec::new();
    };
    let sysfs = sysfs_path_to_name();
    let mut identities = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(stem) = name.to_str().filter(|s| s.starts_with("event")) else {
            continue;
        };
        let path = format!("/dev/input/{stem}");
        let fd = unsafe {
            libc::open(
                std::ffi::CString::new(path.clone()).unwrap().as_ptr(),
                libc::O_RDONLY | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            continue;
        }
        let has_letters = key_bitmap(fd).is_some_and(|b| has_letter_keys(&b));
        unsafe { libc::close(fd) };
        if !has_letters {
            continue;
        }
        let devpath = std::fs::canonicalize(format!("/sys/class/input/{stem}/device"))
            .ok()
            .and_then(|path| path.to_str().map(normalize_sysfs_path))
            .unwrap_or_default();
        let identity_name = sysfs
            .iter()
            .find(|(path, _)| *path == devpath)
            .map(|(_, name)| name.clone())
            .unwrap_or_default();
        identities.push(KeyboardIdentity {
            path: devpath,
            name: identity_name,
        });
    }
    identities
}

/// Canonicalize a sysfs path to the absolute `/devices/...` form used by
/// `/proc/bus/input/devices`.
///
/// `std::fs::canonicalize("/sys/class/input/event0/device")` already resolves
/// the symlink to `/sys/devices/platform/i8042/serio0/input/input0`, but callers
/// compare paths across sources and `/proc` uses the `/devices`-rooted spelling,
/// so the `/sys` prefix is stripped to make the two forms comparable.
pub fn normalize_sysfs_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("/sys") {
        let rest = rest.strip_prefix("/devices").unwrap_or(rest);
        return format!("/devices{rest}");
    }
    path.to_string()
}

/// Map `/devices/...` sysfs paths to evdev device names using
/// `/proc/bus/input/devices`, which is generated from the same kernel data.
fn sysfs_path_to_name() -> Vec<(String, String)> {
    let Ok(devices) = fs::read_to_string("/proc/bus/input/devices") else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    for block in devices.split("\n\n") {
        let mut path = None;
        let mut name = None;
        for line in block.lines() {
            if let Some(sysfs) = line.strip_prefix("S: Sysfs=") {
                path = Some(sysfs.trim().to_string());
            } else if let Some(n) = line.strip_prefix("N: Name=") {
                name = Some(n.trim().trim_matches('"').to_string());
            }
        }
        if let (Some(path), Some(name)) = (path, name) {
            pairs.push((path, name));
        }
    }
    pairs
}

/// Fallback: parse `/proc/bus/input/devices` and apply the same letter-key
/// test to the `B: KEY` bitmap of each `kbd` device block. The bitmap is
/// printed as space-separated 8-byte hex words, most-significant byte of
/// each word first (i.e. the word is byte-reversed vs. memory order).
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
        || !rest
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b':')
        || rest.contains(':')
    {
        return None;
    }
    Some((bus, port))
}

/// Identity of a letter-capable keyboard: its kernel device path and name.
///
/// Source paths come from `/proc/bus/input/devices` (`/devices/...` for evdev
/// probes, `S: Sysfs=` for the `/proc` fallback), so the two detectors can be
/// correlated into one identity set.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyboardIdentity {
    /// Kernel device path, `/devices/...` form.
    pub path: String,
    /// evdev device name from `N: Name=`.
    pub name: String,
}

impl KeyboardIdentity {
    /// True for the built-in `i8042` AT keyboard, which never unregisters on
    /// 2-in-1 laptops even when the folio is detached.
    ///
    /// Only `serio0` (the internal `isa0060` controller) is built into the
    /// machine. `serio1` and up are the external PS/2 ports of a docking
    /// station and *do* disappear when the dock is removed, so they keep
    /// counting as external keyboards.
    pub fn is_internal(&self) -> bool {
        self.path.starts_with("/devices/platform/i8042/serio0")
    }

    /// True for keyboard-capable APCI/WMI hotkey arrays, buttons and other
    /// event devices. These expose letter bits but cannot type text, and they
    /// never disappear on detach.
    pub fn is_auxiliary(&self) -> bool {
        self.path.starts_with("/devices/platform/INT")
            || self.path.starts_with("/devices/platform/PNP")
            || self.path.starts_with("/devices/platform/wmi")
            || self.path.contains("/wmi_bus/")
            || self.name.starts_with("Intel HID")
    }

    /// True when this device is part of the machine rather than something the
    /// user plugged in. Only `is_external_typing_keyboard` decides attachment.
    pub fn is_built_in(&self) -> bool {
        self.is_internal() || self.is_auxiliary()
    }
}

/// True when the device is a keyboard the user can actually type on and that
/// is expected to disappear once unplugged.
///
/// The letter-key probe is necessary but not sufficient: on every 2-in-1
/// laptop the built-in `i8042` keyboard keeps its `KEY_A..KEY_Z` bits forever,
/// and this Dell's `INT33D5:00` "Intel HID events" / "5 button array" devices
/// carry letter bits too (the `0x1e...` PNP0C32 dispatch words). Treating any
/// of them as an attached keyboard would suppress auto-show permanently.
pub fn is_external_typing_keyboard(identity: &KeyboardIdentity) -> bool {
    !identity.is_built_in()
}

/// Every letter-capable keyboard, merged from the evdev probe (when
/// `/dev/input` is readable) and the `/proc` fallback, deduplicated by path so
/// the volatile `/proc` data never reports a phantom detach.
pub fn letter_keyboard_identities() -> Vec<KeyboardIdentity> {
    let evdev = evdev_letter_keyboards();
    let mut identities = evdev.clone();
    if evdev.is_empty() {
        identities.extend(proc_letter_keyboards());
    }
    identities.sort();
    identities.dedup();
    identities
}

/// True when a letter-capable keyboard that is not built into the machine is
/// present, e.g. an external USB keyboard or a detached-USB folio.
pub fn external_keyboard_present() -> bool {
    letter_keyboard_identities()
        .iter()
        .any(is_external_typing_keyboard)
}

/// True when the built-in `i8042` or auxiliary ACPI/WMI keyboard devices expose
/// letter keys. Kept for diagnostics: this is *always* true on a 2-in-1 laptop.
pub fn builtin_keyboard_present() -> bool {
    letter_keyboard_identities()
        .iter()
        .any(|id| id.is_built_in())
}

/// Parse one `/proc/bus/input/devices` block into a keyboard identity.
fn parse_proc_keyboard_block(block: &str) -> Option<KeyboardIdentity> {
    let mut has_kbd = false;
    let mut path = None;
    let mut name = String::new();
    let mut key_bits = None;

    for line in block.lines() {
        if let Some(handlers) = line.strip_prefix("H: ") {
            has_kbd = handlers.split_whitespace().any(|h| h == "kbd");
        } else if let Some(sysfs) = line.strip_prefix("S: Sysfs=") {
            path = Some(sysfs.trim().to_string());
        } else if let Some(n) = line.strip_prefix("N: Name=") {
            name = n.trim().trim_matches('"').to_string();
        } else if let Some(bitmap) = line.strip_prefix("B: KEY=") {
            let mut bits = [0u8; KEY_BITMAP_BYTES];
            let mut consumed = 0;
            for word in bitmap.split_whitespace() {
                if consumed >= bits.len() {
                    break;
                }
                let Ok(word) = u64::from_str_radix(word, 16) else {
                    continue;
                };
                let bytes = word.to_le_bytes();
                for k in 0..8 {
                    if consumed + k < bits.len() {
                        // /proc prints each 8-byte word most-significant byte
                        // first, i.e. byte-reversed vs. memory order.
                        bits[consumed + k] = bytes[7 - k];
                    }
                }
                consumed += 8;
            }
            key_bits = Some(bits);
        }
    }

    if !has_kbd {
        return None;
    }
    let path = path?;
    if !has_letter_keys(&key_bits?) {
        return None;
    }
    Some(KeyboardIdentity { path, name })
}

/// Enumerate letter-capable keyboards from `/proc/bus/input/devices`.
pub fn proc_letter_keyboards() -> Vec<KeyboardIdentity> {
    let Ok(devices) = fs::read_to_string("/proc/bus/input/devices") else {
        return Vec::new();
    };
    devices
        .split("\n\n")
        .filter_map(parse_proc_keyboard_block)
        .collect()
}

/// True if any `kbd` device in `/proc/bus/input/devices` can type letters.
///
/// This includes the built-in `i8042` keyboard and is therefore always true on
/// a 2-in-1 laptop; prefer [`external_keyboard_present`] for gating decisions.
pub fn proc_keyboard_present() -> bool {
    !proc_letter_keyboards().is_empty()
}

/// True if a readable evdev device can type letters.
///
/// Includes the built-in `i8042` keyboard; prefer
/// [`external_keyboard_present`] for gating decisions.
pub fn evdev_keyboard_present() -> bool {
    !evdev_letter_keyboards().is_empty()
}
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
    // Intel HID switches on INT33D5:00, Alps Touchpad on usb1 port 4).
    //
    // The built-in i8042 "AT Translated Set 2 keyboard"
    // (/devices/platform/i8042/serio0/input/input0) and the INT33D5:00
    // "Intel HID events" / "Intel HID 5 button array" utilities carry
    // KEY_A..KEY_Z forever. They are reported as `builtin_keyboard` and must
    // never be read as an attached keyboard.
    //
    // | folio    | posture switch    | 044e:1218 | external kbd  | result   |
    // |----------|-------------------|-----------|---------------|----------|
    // | docked   | inactive          | yes       | no            | attached |
    // | docked   | stuck on (resume) | yes       | no            | attached |
    // | folded   | active            | yes       | no            | attached |
    // | detached | active            | no        | no            | DETACHED |
    // | detached | stuck inactive    | no        | no            | detached |
    // | detached | active            | no        | yes (USB kbd) | attached |

    #[test]
    fn detached_folio_is_detected_even_though_i8042_never_unregisters() {
        // The regression that must never come back: tablet_mode active,
        // i8042 still exposing letter keys, folio gone -> detached.
        assert_eq!(
            verdict(true, false, false, true),
            (false, FolioSource::PostureSwitch)
        );
    }

    #[test]
    fn detached_folio_with_stale_inactive_switch_reports_none() {
        assert_eq!(
            verdict(false, false, false, true),
            (false, FolioSource::None)
        );
    }

    #[test]
    fn missing_builtin_keyboard_still_reports_detached() {
        // An uninitialized i8042 right after resume must not flip the verdict;
        // it only means the switch is trusted with less corroboration.
        assert_eq!(
            verdict(true, false, false, false),
            (false, FolioSource::PostureSwitch)
        );
    }

    #[test]
    fn usb_folio_wins_over_an_inactive_posture_switch() {
        // 360-degree fold where the firmware never sent 0xcc: the USB ID is the
        // only evidence left and it looks exactly like laptop posture. Prefer
        // attached so folding the folio cannot re-enable auto-show.
        let (attached, source) = verdict(false, true, false, true);
        assert!(attached);
        assert_eq!(source, FolioSource::UsbFolio);
    }

    #[test]
    fn usb_folio_wins_over_a_stuck_active_posture_switch() {
        // Laptop posture after resume: the alps device never resent
        // SW_TABLET_MODE=0, so the switch stays latched on.
        let (attached, source) = verdict(true, true, false, true);
        assert!(attached);
        assert_eq!(source, FolioSource::UsbFolio);
    }

    #[test]
    fn external_keyboard_counts_even_with_a_stuck_posture_switch() {
        // A keyboard that disappears on unplug is real typing hardware.
        assert_eq!(
            verdict(true, false, true, true),
            (true, FolioSource::EvdevKeyboard)
        );
        assert_eq!(
            verdict(false, false, true, true),
            (true, FolioSource::EvdevKeyboard)
        );
    }

    #[test]
    fn dock_ps2_port_is_external_but_serio0_is_not() {
        // Both are letter-capable i8042 ports, but only serio0 survives a dock
        // removal, so only serio0 may be treated as built-in.
        let on_board = KeyboardIdentity {
            path: "/devices/platform/i8042/serio0/input/input0".to_string(),
            name: "AT Translated Set 2 keyboard".to_string(),
        };
        let dock_port = KeyboardIdentity {
            path: "/devices/platform/i8042/serio1/input/input8".to_string(),
            name: "AT Translated Set 2 keyboard".to_string(),
        };
        assert!(on_board.is_built_in());
        assert!(!is_external_typing_keyboard(&on_board));
        assert!(!dock_port.is_built_in());
        assert!(is_external_typing_keyboard(&dock_port));
    }

    #[test]
    fn builtin_keyboard_alone_never_produces_an_attachment() {
        // i8042/ACPI/WMI letter bits with no external keyboard and no USB folio
        // must never gate auto-show, or every 2-in-1 would suppress forever.
        assert!(!verdict(true, false, false, true).0);
        assert!(!verdict(false, false, false, true).0);
    }

    #[test]
    fn internal_keyboard_is_classified_as_built_in() {
        let i8042 = KeyboardIdentity {
            path: "/devices/platform/i8042/serio0/input/input0".to_string(),
            name: "AT Translated Set 2 keyboard".to_string(),
        };
        assert!(i8042.is_internal());
        assert!(i8042.is_built_in());
        assert!(!is_external_typing_keyboard(&i8042));
    }

    #[test]
    fn acpi_and_wmi_hotkey_arrays_are_classified_as_built_in() {
        let int_hid = KeyboardIdentity {
            path: "/devices/platform/INT33D5:00/input/input1".to_string(),
            name: "Intel HID events".to_string(),
        };
        let button_array = KeyboardIdentity {
            path: "/devices/platform/INT33D5:00/input/input5".to_string(),
            name: "Intel HID 5 button array".to_string(),
        };
        let wmi = KeyboardIdentity {
            path: "/devices/platform/PNP0C14:03/wmi_bus/wmi_bus-9DBB/input/input6".to_string(),
            name: "Dell WMI hotkeys".to_string(),
        };
        for identity in [int_hid, button_array, wmi] {
            assert!(identity.is_built_in(), "{} must be built-in", identity.path);
            assert!(!is_external_typing_keyboard(&identity));
        }
    }

    #[test]
    fn usb_and_platform_keyboards_are_classified_as_external() {
        let usb = KeyboardIdentity {
            path:
                "/devices/pci0000:00/0000:00:14.0/usb1/1-2/1-2:1.0/0003:1234:5678.0001/input/input9"
                    .to_string(),
            name: "Logitech USB Keyboard".to_string(),
        };
        let at_translated_on_ps2_port = KeyboardIdentity {
            path: "/devices/platform/i8042/serio1/input/input8".to_string(),
            name: "AT Translated Set 2 keyboard".to_string(),
        };
        assert!(is_external_typing_keyboard(&usb));
        // serio1 is the docking-station PS/2 port: it really does disappear
        // when the dock is removed, unlike serio0.
        assert!(is_external_typing_keyboard(&at_translated_on_ps2_port));
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
            external_keyboard: false,
            builtin_keyboard: true,
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
    fn proc_parser_identifies_the_i8042_keyboard_block() {
        // Verbatim /proc/bus/input/devices entry from the target machine.
        let block = "I: Bus=0011 Vendor=0001 Product=0001 Version=ab41\n\
                     N: Name=\"AT Translated Set 2 keyboard\"\n\
                     P: Phys=isa0060/serio0/input0\n\
                     S: Sysfs=/devices/platform/i8042/serio0/input/input0\n\
                     U: Uniq=\n\
                     H: Handlers=sysrq kbd leds event0 \n\
                     B: PROP=0\n\
                     B: EV=120013\n\
                     B: KEY=402000000 3803078f800d001 feffffdfffefffff fffffffffffffffe";
        let identity = parse_proc_keyboard_block(block).expect("block must parse");
        assert_eq!(identity.name, "AT Translated Set 2 keyboard");
        assert_eq!(identity.path, "/devices/platform/i8042/serio0/input/input0");
        assert!(identity.is_internal());
        assert!(!is_external_typing_keyboard(&identity));
    }

    #[test]
    fn proc_parser_ignores_non_keyboard_kbd_handlers() {
        // "Intel HID switches" has a kbd-less handler list and SW bits only.
        let block = "I: Bus=0019 Vendor=0000 Product=0000 Version=0000\n\
                     N: Name=\"Intel HID switches\"\n\
                     P: Phys=\n\
                     S: Sysfs=/devices/platform/INT33D5:00/input/input29\n\
                     H: Handlers=event14 \n\
                     B: PROP=0\n\
                     B: EV=21\n\
                     B: SW=2";
        assert!(parse_proc_keyboard_block(block).is_none());
    }

    #[test]
    fn builtin_keyboards_found_on_this_machine_are_never_external() {
        // Live cross-check: built-in internal/platform keyboards (i8042, ACPI/WMI)
        // must always be classified as built-in, never external. Hot-pluggable
        // USB keyboards/mice are external and are expected on workstations/laptops.
        for identity in letter_keyboard_identities() {
            if identity.path.contains("/usb") {
                assert!(
                    !identity.is_built_in(),
                    "USB keyboard should be external: {:?}",
                    identity
                );
            } else {
                assert!(
                    identity.is_built_in(),
                    "unexpected external keyboard detected: {:?}",
                    identity
                );
            }
        }
    }

    #[test]
    fn sysfs_paths_are_normalized_to_the_devices_form() {
        // `canonicalize` yields `/sys/devices/...`; `/proc/bus/input/devices`
        // prints `/devices/...`. Both must collapse to one comparable string or
        // the merge reports a phantom detach as soon as `/dev/input` becomes
        // unreadable.
        assert_eq!(
            normalize_sysfs_path("/sys/devices/platform/i8042/serio0/input/input0"),
            "/devices/platform/i8042/serio0/input/input0"
        );
        assert_eq!(
            normalize_sysfs_path("/devices/platform/i8042/serio0/input/input0"),
            "/devices/platform/i8042/serio0/input/input0"
        );
        assert_eq!(
            normalize_sysfs_path(
                "/sys/devices/pci0000:00/0000:00:14.0/usb1/1-4/1-4:1.0/input/input12"
            ),
            "/devices/pci0000:00/0000:00:14.0/usb1/1-4/1-4:1.0/input/input12"
        );
    }

    #[test]
    fn canonicalized_live_paths_agree_with_proc() {
        // Live cross-check that the symlink resolution strategy really produces
        // paths in the same form as /proc/bus/input/devices on this kernel.
        // Must compare against *all* /proc sysfs paths, not the filtered
        // letter-capable list: devices like "Intel HID switches" have no kbd
        // handler and are intentionally absent from the filtered set.
        let Ok(entries) = fs::read_dir("/dev/input") else {
            return;
        };
        let proc_paths: Vec<String> = sysfs_path_to_name()
            .into_iter()
            .map(|(path, _)| path)
            .collect();
        if proc_paths.is_empty() {
            return;
        }
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(stem) = name.to_str().filter(|s| s.starts_with("event")) else {
                continue;
            };
            let Ok(resolved) = std::fs::canonicalize(format!("/sys/class/input/{stem}/device"))
            else {
                continue;
            };
            let Some(resolved) = resolved.to_str().map(normalize_sysfs_path) else {
                continue;
            };
            assert!(
                proc_paths.contains(&resolved),
                "canonicalized {stem} path {resolved} not found in /proc/bus/input/devices"
            );
        }
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
