//! Physical keyboard (folio) detection.
//!
//! Hyprosk normally auto-shows on text-input focus. In folio mode
//! ([`crate::config::BehaviorConfig::folio_mode`]) it only does so while no
//! physical keyboard — e.g. an attached laptop/tablet folio — is present.
//!
//! Detection reads `/proc/bus/input/devices`, which lists every evdev input
//! device with its `H:` handlers. A keyboard is present when any device block
//! lists the `kbd` handler. This avoids requiring read access to
//! `/dev/input/event*` (root/input group) and covers USB, PS/2 and Bluetooth
//! folio keyboards alike. The zwp_virtual_keyboard_v1 interface we use does
//! not create an evdev device, so the on-screen keyboard cannot detect
//! itself.

use std::fs;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

/// Seconds between folio presence polls.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Returns true when at least one physical keyboard is present.
pub fn physical_keyboard_attached() -> bool {
    let devices = match fs::read_to_string("/proc/bus/input/devices") {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("Cannot read /proc/bus/input/devices, assuming no folio keyboard: {e}");
            return false;
        }
    };

    devices
        .split("\n\n")
        .any(|block| block.lines().any(|l| l.starts_with("H:") && l.split_whitespace().any(|h| h == "kbd")))
}

/// Spawns a background thread that polls folio presence every
/// [`POLL_INTERVAL`] and sends each *change* through `tx` (`true` = attached,
/// `false` = detached).
pub fn spawn_folio_poller(tx: Sender<bool>) {
    thread::spawn(move || {
        let mut last = physical_keyboard_attached();
        loop {
            thread::sleep(POLL_INTERVAL);
            let now = physical_keyboard_attached();
            if now != last {
                let _ = tx.send(now);
                last = now;
            }
        }
    });
}
