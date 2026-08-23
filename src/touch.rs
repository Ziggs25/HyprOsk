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

fn eviocgbit(ev: u32, len: usize) -> libc::c_ulong {
    (_IOC_READ << 30) | ((len as libc::c_ulong) << 16) | (INPUT_TYPE << 8) | (0x20 + ev) as libc::c_ulong
}

const ABS_BITMAP_BYTES: usize = 16;
const KEY_BITMAP_BYTES: usize = 96;

fn has_touch_cap(fd: libc::c_int) -> bool {
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
        let mut fds: Vec<(libc::c_int, bool)> = Vec::new();
        if let Ok(entries) = fs::read_dir("/dev/input") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(stem) = name.to_str().filter(|s| s.starts_with("event")) else { continue };
                let path = format!("/dev/input/{stem}");
                let cpath = std::ffi::CString::new(path.clone()).unwrap();
                let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
                if fd < 0 { continue; }
                let is_touch = has_touch_cap(fd);
                if is_touch {
                    fds.push((fd, true));
                    tracing::info!("touch poller watching {} (touch)", stem);
                } else {
                    unsafe { libc::close(fd) };
                }
            }
        }
        if fds.is_empty() {
            tracing::warn!("touch poller: no touch devices found, touch_only will be permissive");
            return;
        }
        let mut pollfds: Vec<libc::pollfd> = fds.iter().map(|(fd, _)| libc::pollfd { fd: *fd, events: libc::POLLIN, revents: 0 }).collect();
        let mut buf = [0u8; 64 * std::mem::size_of::<InputEvent>()];
        loop {
            let ret = unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, 500) };
            if ret < 0 {
                thread::sleep(Duration::from_millis(500));
                continue;
            }
            for (i, pfd) in pollfds.iter().enumerate() {
                if pfd.revents & libc::POLLIN != 0 {
                    let fd = fds[i].0;
                    loop {
                        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
                        if n <= 0 { break; }
                        let count = n as usize / std::mem::size_of::<InputEvent>();
                        for j in 0..count {
                            let ev = unsafe { *(buf.as_ptr().add(j * std::mem::size_of::<InputEvent>()) as *const InputEvent) };
                            if ev.type_ == EV_ABS as u16 || (ev.type_ == EV_KEY as u16 && ev.code == BTN_TOUCH as u16 && ev.value == 1) {
                                let _ = tx.send(Instant::now());
                                break;
                            }
                        }
                    }
                }
            }
        }
    });
}
