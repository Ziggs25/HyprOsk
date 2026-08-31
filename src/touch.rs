use std::fs;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};
use libc;

const _IOC_READ: libc::c_ulong = 2;
const INPUT_TYPE: libc::c_ulong = b'E' as libc::c_ulong;
const EV_ABS: u32 = 0x03;
const EV_KEY: u32 = 0x01;
const BTN_TOUCH: usize = 330;

const PROP_BITMAP_BYTES: usize = 8;
const INPUT_PROP_POINTER: usize = 0;
const INPUT_PROP_DIRECT: usize = 1;
const INPUT_PROP_BUTTONPAD: usize = 2;

fn eviocgbit(ev: u32, len: usize) -> libc::c_ulong {
    (_IOC_READ << 30) | ((len as libc::c_ulong) << 16) | (INPUT_TYPE << 8) | (0x20 + ev) as libc::c_ulong
}

fn eviocgprop(len: usize) -> libc::c_ulong {
    (_IOC_READ << 30) | ((len as libc::c_ulong) << 16) | (INPUT_TYPE << 8) | 0x09
}

fn eviocgname(len: usize) -> libc::c_ulong {
    (_IOC_READ << 30) | ((len as libc::c_ulong) << 16) | (INPUT_TYPE << 8) | 0x06
}

const ABS_BITMAP_BYTES: usize = 16;
const KEY_BITMAP_BYTES: usize = 96;

fn has_touch_cap(fd: libc::c_int) -> bool {
    // 1. Exclude devices whose name indicates touchpad/trackpoint/mouse
    let mut name_buf = [0u8; 256];
    let name_ok = unsafe { libc::ioctl(fd, eviocgname(256), name_buf.as_mut_ptr()) } >= 0;
    if name_ok {
        let name = String::from_utf8_lossy(&name_buf).to_lowercase();
        if name.contains("touchpad") || name.contains("trackpoint") || name.contains("mouse") {
            return false;
        }
    }

    // 2. Check input properties: reject pointers/buttonpads, accept direct touchscreens
    let mut prop_bits = [0u8; PROP_BITMAP_BYTES];
    let prop_ok = unsafe { libc::ioctl(fd, eviocgprop(PROP_BITMAP_BYTES), prop_bits.as_mut_ptr()) } >= 0;
    if prop_ok {
        let is_pointer = (prop_bits[INPUT_PROP_POINTER / 8] >> (INPUT_PROP_POINTER % 8)) & 1 != 0;
        let is_buttonpad = (prop_bits[INPUT_PROP_BUTTONPAD / 8] >> (INPUT_PROP_BUTTONPAD % 8)) & 1 != 0;
        if is_pointer || is_buttonpad {
            return false;
        }
        let is_direct = (prop_bits[INPUT_PROP_DIRECT / 8] >> (INPUT_PROP_DIRECT % 8)) & 1 != 0;
        if is_direct {
            return true;
        }
    }

    // 3. Fallback: check ABS multi-touch or BTN_TOUCH
    let mut abs_bits = [0u8; ABS_BITMAP_BYTES];
    let mut key_bits = [0u8; KEY_BITMAP_BYTES];
    let abs_ok = unsafe { libc::ioctl(fd, eviocgbit(EV_ABS, ABS_BITMAP_BYTES), abs_bits.as_mut_ptr()) } >= 0;
    let key_ok = unsafe { libc::ioctl(fd, eviocgbit(EV_KEY, KEY_BITMAP_BYTES), key_bits.as_mut_ptr()) } >= 0;
    if abs_ok {
        let has_mt = abs_bits.iter().any(|b| *b != 0);
        if has_mt {
            let mt_props = [0x2f, 0x30, 0x31, 0x32, 0x33, 0x35, 0x36, 0x39, 0x3a, 53, 54];
            for code in mt_props {
                if (abs_bits[code / 8] >> (code % 8)) & 1 != 0 {
                    return true;
                }
            }
        }
    }
    if key_ok && (key_bits[BTN_TOUCH / 8] >> (BTN_TOUCH % 8)) & 1 != 0 {
        return true;
    }
    false
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    sec: libc::time_t,
    usec: libc::suseconds_t,
    type_: u16,
    code: u16,
    value: i32,
}

pub fn spawn_touch_poller(tx: Sender<Instant>) {
    thread::spawn(move || {
        let mut devices: Vec<(String, libc::c_int)> = Vec::new();

        let scan_devices = |devices: &mut Vec<(String, libc::c_int)>| {
            if let Ok(entries) = fs::read_dir("/dev/input") {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let Some(stem) = name.to_str().filter(|s| s.starts_with("event")) else { continue };
                    let path = format!("/dev/input/{stem}");
                    if devices.iter().any(|(p, _)| p == &path) {
                        continue;
                    }
                    let cpath = match std::ffi::CString::new(path.clone()) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC) };
                    if fd < 0 { continue; }
                    if has_touch_cap(fd) {
                        tracing::info!("touch poller watching {} (touch)", stem);
                        devices.push((path, fd));
                    } else {
                        unsafe { libc::close(fd) };
                    }
                }
            }
        };

        scan_devices(&mut devices);

        let mut buf = [0u8; 64 * std::mem::size_of::<InputEvent>()];
        let mut last_scan = Instant::now();

        loop {
            // If no touch devices are available, wait and scan periodically
            if devices.is_empty() {
                thread::sleep(Duration::from_secs(2));
                scan_devices(&mut devices);
                if devices.is_empty() {
                    continue;
                }
            }

            // Periodically check for newly plugged touch devices every 5 seconds
            if last_scan.elapsed() >= Duration::from_secs(5) {
                scan_devices(&mut devices);
                last_scan = Instant::now();
            }

            let mut pollfds: Vec<libc::pollfd> = devices
                .iter()
                .map(|(_, fd)| libc::pollfd {
                    fd: *fd,
                    events: libc::POLLIN,
                    revents: 0,
                })
                .collect();

            let ret = unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, 1000) };
            if ret < 0 {
                let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                if err != libc::EINTR {
                    thread::sleep(Duration::from_millis(500));
                }
                continue;
            }
            if ret == 0 {
                // Timeout elapsed with no events
                continue;
            }

            let mut dead_indices = Vec::new();

            for (i, pfd) in pollfds.iter().enumerate() {
                if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    dead_indices.push(i);
                    continue;
                }

                if pfd.revents & libc::POLLIN != 0 {
                    let fd = devices[i].1;
                    let mut has_read_error = false;

                    loop {
                        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
                        if n < 0 {
                            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                            if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                                break;
                            } else {
                                has_read_error = true;
                                break;
                            }
                        } else if n == 0 {
                            has_read_error = true;
                            break;
                        }

                        let count = n as usize / std::mem::size_of::<InputEvent>();
                        for j in 0..count {
                            let ev = unsafe {
                                *(buf.as_ptr().add(j * std::mem::size_of::<InputEvent>()) as *const InputEvent)
                            };
                            if ev.type_ == EV_ABS as u16
                                || (ev.type_ == EV_KEY as u16 && ev.code == BTN_TOUCH as u16 && ev.value == 1)
                            {
                                let _ = tx.send(Instant::now());
                                break;
                            }
                        }
                    }

                    if has_read_error {
                        dead_indices.push(i);
                    }
                }
            }

            // Remove and close dead descriptors in reverse order
            dead_indices.sort_unstable();
            dead_indices.dedup();
            for &idx in dead_indices.iter().rev() {
                let (path, fd) = devices.remove(idx);
                tracing::info!("touch poller removed disconnected device {}", path);
                unsafe { libc::close(fd) };
            }
        }
    });
}
