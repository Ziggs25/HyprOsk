use std::collections::{HashSet, VecDeque};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use zbus::blocking::connection::Builder as ConnectionBuilder;
use zbus::blocking::{Connection, Proxy};

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const RESCAN_INTERVAL: Duration = Duration::from_millis(500);
const MAX_TEXT_WINDOW_CHARS: usize = 4096;
const MAX_SCAN_NODES: usize = 5000;
const MAX_SCAN_DEPTH: usize = 32;
const FOCUSED: u32 = 12;
const EDITABLE: u32 = 7;
const TEXT_INTERFACE: &str = "org.a11y.atspi.Text";
const ACCESSIBLE_INTERFACE: &str = "org.a11y.atspi.Accessible";
const ROOT_PATH: &str = "/org/a11y/atspi/accessible/root";
const NULL_PATH: &str = "/org/a11y/atspi/null";
const REGISTRY_NAME: &str = "org.a11y.atspi.Registry";

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Snapshot {
    pub app_pid: u32,
    pub text: String,
    pub cursor_bytes: usize,
}

struct FocusedWindow {
    app_pid: u32,
    title: String,
    identity: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ObjectRef {
    bus_name: String,
    path: String,
}

struct AtspiClient {
    connection: Connection,
    cached_pid: Option<u32>,
    cached_title: Option<String>,
    cached_window_identity: Option<String>,
    focused: Option<ObjectRef>,
    last_scan: Instant,
}

impl AtspiClient {
    fn connect() -> zbus::Result<Self> {
        let session = Connection::session()?;
        let session_proxy = Proxy::new(
            &session,
            "org.a11y.Bus",
            "/org/a11y/bus",
            "org.a11y.Bus",
        )?;
        let address: String = session_proxy.call("GetAddress", &())?;
        let connection = ConnectionBuilder::address(address.as_str())?.build()?;
        Ok(Self {
            connection,
            cached_pid: None,
            cached_title: None,
            cached_window_identity: None,
            focused: None,
            last_scan: Instant::now() - RESCAN_INTERVAL,
        })
    }

    fn sample(&mut self, app_pid: u32, app_title: &str, window_identity: &str) -> Option<Snapshot> {
        if self.cached_pid != Some(app_pid)
            || self.cached_title.as_deref() != Some(app_title)
            || self.cached_window_identity.as_deref() != Some(window_identity)
        {
            self.cached_pid = Some(app_pid);
            self.cached_title = Some(app_title.to_owned());
            self.cached_window_identity = Some(window_identity.to_owned());
            self.focused = None;
            self.last_scan = Instant::now() - RESCAN_INTERVAL;
        }

        if let Some(object) = self.focused.clone() {
            match read_focused_text(&self.connection, &object, app_pid) {
                Ok(Some(snapshot)) => return Some(snapshot),
                Ok(None) | Err(_) => self.focused = None,
            }
        }

        if self.last_scan.elapsed() < RESCAN_INTERVAL {
            return None;
        }
        self.focused = find_focused_editable(&self.connection, app_pid, app_title);
        self.last_scan = Instant::now();
        self.focused
            .as_ref()
            .and_then(|object| read_focused_text(&self.connection, object, app_pid).ok().flatten())
    }

    fn clear_focused(&mut self) {
        self.focused = None;
        self.last_scan = Instant::now() - RESCAN_INTERVAL;
    }
}

pub fn spawn_worker(sender: SyncSender<Snapshot>, input_active: Arc<AtomicBool>) {
    let _ = thread::Builder::new()
        .name("hyprosk-atspi".into())
        .spawn(move || {
            let mut client: Option<AtspiClient> = None;
            let mut last_pid = None;
            let mut last_window_identity: Option<String> = None;
            let mut last_sent: Option<Snapshot> = None;
            let mut last_sent_at: Option<Instant> = None;
            let mut last_connect_warning = Instant::now() - Duration::from_secs(30);
            loop {
                if !input_active.load(Ordering::Relaxed) {
                    client = None;
                    last_pid = None;
                    last_window_identity = None;
                    last_sent = None;
                    last_sent_at = None;
                    thread::sleep(POLL_INTERVAL);
                    continue;
                }

                let Some(window) = focused_window() else {
                    last_pid = None;
                    last_window_identity = None;
                    last_sent = None;
                    last_sent_at = None;
                    if let Some(client) = client.as_mut() {
                        client.clear_focused();
                    }
                    thread::sleep(POLL_INTERVAL);
                    continue;
                };
                if last_pid != Some(window.app_pid)
                    || last_window_identity.as_deref() != Some(window.identity.as_str())
                {
                    last_pid = Some(window.app_pid);
                    last_window_identity = Some(window.identity.clone());
                    last_sent = None;
                    last_sent_at = None;
                }

                if client.is_none() {
                    match AtspiClient::connect() {
                        Ok(connected) => {
                            tracing::info!("AT-SPI fallback connected");
                            client = Some(connected);
                        }
                        Err(error) => {
                            if last_connect_warning.elapsed() >= Duration::from_secs(30) {
                                tracing::debug!(error = %error, "AT-SPI fallback unavailable");
                                last_connect_warning = Instant::now();
                            }
                            thread::sleep(Duration::from_secs(2));
                            continue;
                        }
                    }
                }

                if let Some(snapshot) = client
                    .as_mut()
                    .and_then(|client| client.sample(window.app_pid, &window.title, &window.identity))
                {
                    let changed = last_sent.as_ref() != Some(&snapshot);
                    let refresh_due = last_sent_at
                        .is_none_or(|last| last.elapsed() >= Duration::from_secs(1));
                    if changed || refresh_due {
                        match sender.try_send(snapshot.clone()) {
                            Ok(()) => {
                                last_sent = Some(snapshot);
                                last_sent_at = Some(Instant::now());
                            }
                            Err(TrySendError::Full(_)) => {}
                            Err(TrySendError::Disconnected(_)) => return,
                        }
                    }
                }
                thread::sleep(POLL_INTERVAL);
            }
        });
}
fn focused_window() -> Option<FocusedWindow> {
    let niri_session = std::env::var_os("NIRI_SOCKET").is_some();
    let commands: &[(&str, &[&str])] = if niri_session {
        &[("niri", &["msg", "--json", "focused-window"]), ("hyprctl", &["activewindow", "-j"])]
    } else {
        &[("hyprctl", &["activewindow", "-j"]), ("niri", &["msg", "--json", "focused-window"])]
    };

    for (program, args) in commands {
        let Ok(output) = Command::new(program).args(*args).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) else {
            continue;
        };
        if let Some(pid) = value.get("pid").and_then(json_pid)
            && pid > 0
        {
            let title = value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let raw_identity = value
                .get("id")
                .and_then(|id| id.as_u64().map(|id| id.to_string()).or_else(|| id.as_str().map(str::to_owned)))
                .or_else(|| value.get("address").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_else(|| title.clone());
            return Some(FocusedWindow {
                app_pid: pid,
                title,
                identity: format!("{program}:{raw_identity}"),
            });
        }
    }
    None
}

fn json_pid(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .or_else(|| value.as_str()?.parse().ok())
}

fn find_focused_editable(
    connection: &Connection,
    app_pid: u32,
    app_title: &str,
) -> Option<ObjectRef> {
    let scan_started = Instant::now();
    let root = ObjectRef {
        bus_name: REGISTRY_NAME.to_owned(),
        path: ROOT_PATH.to_owned(),
    };
    let children = match get_children(connection, &root) {
        Ok(children) => children,
        Err(error) => {
            tracing::debug!(error = %error, "AT-SPI could not enumerate registry applications");
            return None;
        }
    };
    tracing::debug!(root_children = children.len(), target_pid = app_pid, "AT-SPI registry scan started");
    let mut matched_apps = 0usize;
    let mut frame_count = 0usize;
    let mut selected_frame_count = 0usize;
    let mut fallback_app_count = 0usize;
    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();

    for app in children {
        if app.path == NULL_PATH || app.bus_name.is_empty() {
            continue;
        }
        let Ok(pid) = connection_pid(connection, &app.bus_name) else {
            continue;
        };
        if pid != app_pid {
            continue;
        }
        matched_apps += 1;
        let Ok(frames) = get_children(connection, &app) else {
            continue;
        };
        frame_count += frames.len();
        let mut fallback_frames = Vec::new();
        let mut matched_frame = false;
        let mut selected_frames = 0usize;
        for frame in frames {
            if frame.path == NULL_PATH || frame.bus_name.is_empty() {
                continue;
            }
            fallback_frames.push(frame.clone());
            let _: zbus::Result<std::collections::HashMap<String, String>> =
                proxy(connection, &frame, ACCESSIBLE_INTERFACE)
                    .and_then(|accessible| accessible.call("GetAttributes", &()));
            if !app_title.is_empty()
                && !proxy(connection, &frame, ACCESSIBLE_INTERFACE)
                    .and_then(|accessible| accessible.get_property::<String>("Name"))
                    .is_ok_and(|name| name.starts_with(app_title))
            {
                continue;
            }
            matched_frame = true;
            selected_frames += 1;
            queue.push_back((frame, 1usize));
        }
        if !matched_frame {
            fallback_app_count += 1;
            queue.extend(fallback_frames.into_iter().map(|frame| (frame, 1usize)));
        } else {
            selected_frame_count += selected_frames;
        }
    }
    tracing::debug!(matched_apps, frame_count, selected_frame_count, fallback_app_count, "AT-SPI application frames selected");

    let mut scanned = 0usize;
    let mut focused_nodes = 0usize;
    let mut focused_editable_nodes = 0usize;
    while let Some((object, depth)) = queue.pop_front() {
        if depth > MAX_SCAN_DEPTH || scanned >= MAX_SCAN_NODES || !visited.insert(object.clone()) {
            continue;
        }
        scanned += 1;

        let Ok(states) = get_states(connection, &object) else {
            continue;
        };
        let is_focused = state_is_set(&states, FOCUSED);
        let is_editable = state_is_set(&states, EDITABLE);
        if is_focused {
            focused_nodes += 1;
            if is_editable {
                focused_editable_nodes += 1;
            }
        }
        if is_focused && is_editable_text(connection, &object, &states) {
            tracing::info!(
                scanned_nodes = scanned,
                scan_ms = scan_started.elapsed().as_millis(),
                "AT-SPI discovered focused editable field"
            );
            return Some(object);
        }

        if depth < MAX_SCAN_DEPTH
            && let Ok(children) = get_children(connection, &object)
        {
            queue.extend(
                children
                    .into_iter()
                    .filter(|child| child.path != NULL_PATH && !child.bus_name.is_empty())
                    .map(|child| (child, depth + 1)),
            );
        }
    }
    tracing::debug!(
        scanned_nodes = scanned,
        focused_nodes,
        focused_editable_nodes,
        scan_ms = scan_started.elapsed().as_millis(),
        "AT-SPI scan completed without a focused editable field"
    );
    None
}

fn state_is_set(states: &[u32], state: u32) -> bool {
    states
        .get((state / 32) as usize)
        .is_some_and(|word| word & (1_u32 << (state % 32)) != 0)
}

fn is_editable_text(connection: &Connection, object: &ObjectRef, states: &[u32]) -> bool {
    if !state_is_set(states, EDITABLE) {
        return false;
    }
    let Ok(text_proxy) = proxy(connection, object, TEXT_INTERFACE) else {
        return false;
    };
    let Ok(role): zbus::Result<String> = proxy(connection, object, ACCESSIBLE_INTERFACE)
        .and_then(|accessible| accessible.call("GetRoleName", &()))
    else {
        return false;
    };
    let Ok(attributes): zbus::Result<std::collections::HashMap<String, String>> = proxy(
        connection,
        object,
        ACCESSIBLE_INTERFACE,
    )
    .and_then(|accessible| accessible.call("GetAttributes", &()))
    else {
        return false;
    };
    !is_sensitive(&role, &attributes)
        && text_proxy
            .get_property::<i32>("CharacterCount")
            .is_ok()
}

fn is_sensitive(role: &str, attributes: &std::collections::HashMap<String, String>) -> bool {
    if role.to_ascii_lowercase().contains("password") {
        return true;
    }
    attributes.iter().any(|(key, value)| {
        let key = key.to_ascii_lowercase();
        let value = value.to_ascii_lowercase();
        value == "password"
            || key.contains("password")
            || (key.contains("sensitive") && !matches!(value.as_str(), "" | "0" | "false" | "no"))
            || (matches!(key.as_str(), "type" | "input-type" | "text-input-type" | "html-input-type")
                && matches!(value.as_str(), "password" | "secret"))
    })
}

fn read_focused_text(
    connection: &Connection,
    object: &ObjectRef,
    app_pid: u32,
) -> zbus::Result<Option<Snapshot>> {
    let states = get_states(connection, object)?;
    if !state_is_set(&states, FOCUSED) || !is_editable_text(connection, object, &states) {
        return Ok(None);
    }

    let text_proxy = proxy(connection, object, TEXT_INTERFACE)?;
    let character_count: i32 = text_proxy.get_property("CharacterCount")?;
    let caret: i32 = text_proxy.get_property("CaretOffset")?;
    if character_count < 0 || caret < 0 || caret > character_count {
        return Ok(None);
    }

    let count = character_count as usize;
    let caret = caret as usize;
    let half_window = MAX_TEXT_WINDOW_CHARS / 2;
    let start = caret.saturating_sub(half_window);
    let end = count.min(start.saturating_add(MAX_TEXT_WINDOW_CHARS));
    let start_i32 = i32::try_from(start).unwrap_or(i32::MAX);
    let end_i32 = i32::try_from(end).unwrap_or(i32::MAX);
    let text: String = text_proxy.call("GetText", &(start_i32, end_i32))?;
    let cursor_char = caret.saturating_sub(start);
    let Some(cursor_bytes) = character_offset_to_byte(&text, cursor_char) else {
        return Ok(None);
    };

    Ok(Some(Snapshot {
        app_pid,
        text,
        cursor_bytes,
    }))
}

fn character_offset_to_byte(text: &str, offset: usize) -> Option<usize> {
    if offset == 0 {
        return Some(0);
    }
    text.char_indices()
        .nth(offset)
        .map(|(byte, _)| byte)
        .or_else(|| (text.chars().count() == offset).then_some(text.len()))
}

fn connection_pid(connection: &Connection, bus_name: &str) -> zbus::Result<u32> {
    let dbus = Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )?;
    dbus.call("GetConnectionUnixProcessID", &(bus_name,))
}

fn get_states(connection: &Connection, object: &ObjectRef) -> zbus::Result<Vec<u32>> {
    proxy(connection, object, ACCESSIBLE_INTERFACE)?.call("GetState", &())
}

fn get_children(connection: &Connection, object: &ObjectRef) -> zbus::Result<Vec<ObjectRef>> {
    let children: Vec<(String, zbus::zvariant::OwnedObjectPath)> =
        proxy(connection, object, ACCESSIBLE_INTERFACE)?.call("GetChildren", &())?;
    Ok(children
        .into_iter()
        .map(|(bus_name, path)| ObjectRef {
            bus_name,
            path: path.as_str().to_owned(),
        })
        .collect())
}

fn proxy<'a>(
    connection: &Connection,
    object: &'a ObjectRef,
    interface: &'a str,
) -> zbus::Result<Proxy<'a>> {
    Proxy::new(connection, object.bus_name.as_str(), object.path.as_str(), interface)
}
