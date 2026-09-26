use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;

use crate::layout::LayoutMode;

#[derive(Debug, Clone, PartialEq)]
pub enum IpcCommand {
    Show,
    ShowMode(bool),
    ShowWithMode(LayoutMode),
    Hide,
    Toggle,
    ToggleMode(bool),
    ToggleWithMode(LayoutMode),
    ToggleExclusivity,
    SetExclusivity(bool),
    SetLayoutMode(LayoutMode),
    Reload,
    LoadLayout(String),
    SwitchLayer(String),
    Clipboard,
    Quit,
}

pub struct IpcServer;

impl IpcServer {
    pub fn get_socket_path() -> PathBuf {
        if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
            PathBuf::from(runtime).join("hyprosk.sock")
        } else {
            PathBuf::from("/tmp/hyprosk.sock")
        }
    }

    pub fn send_command(cmd: &str) -> anyhow::Result<String> {
        let socket_path = Self::get_socket_path();
        let mut stream = UnixStream::connect(&socket_path)?;
        stream.write_all(format!("{}\n", cmd.trim()).as_bytes())?;
        stream.flush()?;

        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf[..n]).to_string())
    }

    /// Start the daemon's IPC endpoint and claim the single-daemon socket.
    ///
    /// Binding before connecting to Wayland prevents a second HyprOsk process
    /// from reaching `zwp_input_method_manager_v2` and receiving the protocol's
    /// `unavailable` event because another daemon already owns the seat.
    pub fn start_server(tx: Sender<IpcCommand>) -> anyhow::Result<()> {
        let socket_path = Self::get_socket_path();
        let listener = bind_socket(&socket_path)?;

        thread::spawn(move || {
            tracing::info!("HyprOsk IPC server listening at {:?}", socket_path);

            for stream in listener.incoming() {
                match stream {
                    Ok(mut s) => {
                        let mut buf = [0u8; 256];
                        if let Ok(n) = s.read(&mut buf) {
                            if n == 0 {
                                continue;
                            }
                            let text = String::from_utf8_lossy(&buf[..n]).trim().to_string();
                            let (resp, cmd) = match text.as_str() {
                                "show" => ("OK: Shown\n", Some(IpcCommand::Show)),
                                "show-mobile" | "show mobile" | "show --mobile" => {
                                    ("OK: Shown Mobile\n", Some(IpcCommand::ShowWithMode(LayoutMode::Mobile)))
                                }
                                "show-desktop" | "show desktop" | "show --desktop" => {
                                    ("OK: Shown Desktop\n", Some(IpcCommand::ShowWithMode(LayoutMode::Desktop)))
                                }
                                "show-tiled" | "show tiled" | "show --tiled" | "tiled" => {
                                    ("OK: Shown Tiled\n", Some(IpcCommand::ShowMode(true)))
                                }
                                "show-floating" | "show floating" | "show --floating" | "show-overlay" | "show overlay" | "show --overlay" | "floating" | "overlay" => {
                                    ("OK: Shown Floating\n", Some(IpcCommand::ShowMode(false)))
                                }
                                "hide" => ("OK: Hidden\n", Some(IpcCommand::Hide)),
                                "toggle" => ("OK: Toggled\n", Some(IpcCommand::Toggle)),
                                "toggle-mobile" | "toggle mobile" | "toggle --mobile" => {
                                    ("OK: Toggled Mobile\n", Some(IpcCommand::ToggleWithMode(LayoutMode::Mobile)))
                                }
                                "toggle-desktop" | "toggle desktop" | "toggle --desktop" => {
                                    ("OK: Toggled Desktop\n", Some(IpcCommand::ToggleWithMode(LayoutMode::Desktop)))
                                }
                                "toggle-tiled" | "toggle tiled" | "toggle --tiled" => {
                                    ("OK: Toggled Tiled\n", Some(IpcCommand::ToggleMode(true)))
                                }
                                "toggle-floating" | "toggle floating" | "toggle --floating" | "toggle-overlay" | "toggle overlay" | "toggle --overlay" => {
                                    ("OK: Toggled Floating\n", Some(IpcCommand::ToggleMode(false)))
                                }
                                "exclusive" | "exclusivity" | "toggle-exclusive" => {
                                    ("OK: Exclusivity Toggled\n", Some(IpcCommand::ToggleExclusivity))
                                }
                                "exclusive on" | "exclusive true" | "tiled on" => {
                                    ("OK: Exclusivity On\n", Some(IpcCommand::SetExclusivity(true)))
                                }
                                "exclusive off" | "exclusive false" | "overlay on" | "floating on" => {
                                    ("OK: Exclusivity Off\n", Some(IpcCommand::SetExclusivity(false)))
                                }
                                "reload" => ("OK: Config Reloaded\n", Some(IpcCommand::Reload)),
                                "clipboard" => ("OK: Clipboard\n", Some(IpcCommand::Clipboard)),
                                "quit" => ("OK: Quitting\n", Some(IpcCommand::Quit)),
                                s if s.starts_with("mode ") => {
                                    let m_name = s.strip_prefix("mode ").unwrap().trim();
                                    if let Some(m) = LayoutMode::parse_mode(m_name) {
                                        ("OK: Mode Switched\n", Some(IpcCommand::SetLayoutMode(m)))
                                    } else {
                                        ("ERR: Unknown mode, expected 'desktop' or 'mobile'\n", None)
                                    }
                                }
                                s if s.starts_with("layout ") => {
                                    let l_name = s.strip_prefix("layout ").unwrap().trim().to_string();
                                    ("OK: Layout Loaded\n", Some(IpcCommand::LoadLayout(l_name)))
                                }
                                s if s.starts_with("layer ") => {
                                    let l_name = s.strip_prefix("layer ").unwrap().trim().to_string();
                                    ("OK: Layer Switched\n", Some(IpcCommand::SwitchLayer(l_name)))
                                }
                                _ => ("ERR: Unknown command\n", None),
                            };

                            let _ = s.write_all(resp.as_bytes());
                            let _ = s.flush();
                            if let Some(c) = cmd {
                                let _ = tx.send(c);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("IPC connection error: {:?}", e);
                    }
                }
            }
        });

        Ok(())
    }
}

/// Bind the daemon socket without stealing it from an already-running daemon.
///
/// A dead Unix socket is removed and retried, which handles a daemon that was
/// terminated without a graceful IPC shutdown. A live socket is reported as a
/// single-instance conflict instead.
fn bind_socket(path: &Path) -> io::Result<UnixListener> {
    match UnixListener::bind(path) {
        Ok(listener) => Ok(listener),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "another HyprOsk daemon is already running",
                ));
            }

            // The endpoint is stale: no process accepted a connection, so it
            // is safe for this new daemon to reclaim the path.
            std::fs::remove_file(path)?;
            UnixListener::bind(path)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::bind_socket;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_socket_path() -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "hyprosk-ipc-test-{}-{}.sock",
            std::process::id(),
            timestamp
        ))
    }

    #[test]
    fn second_daemon_cannot_claim_a_live_ipc_socket() {
        let path = test_socket_path();
        let first = bind_socket(&path).expect("first daemon should claim the socket");

        let second = bind_socket(&path).expect_err("second daemon must be rejected");
        assert_eq!(second.kind(), std::io::ErrorKind::AddrInUse);
        assert!(second.to_string().contains("another HyprOsk daemon"));

        drop(first);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn stale_socket_is_reclaimed() {
        let path = test_socket_path();
        let stale = UnixListener::bind(&path).expect("test should create a socket");
        drop(stale);

        let listener = bind_socket(&path).expect("new daemon should reclaim stale socket");
        drop(listener);
        let _ = std::fs::remove_file(path);
    }
}
