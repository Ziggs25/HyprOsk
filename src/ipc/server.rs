use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;

#[derive(Debug, Clone, PartialEq)]
pub enum IpcCommand {
    Show,
    ShowMode(bool),
    Hide,
    Toggle,
    ToggleMode(bool),
    ToggleExclusivity,
    SetExclusivity(bool),
    Reload,
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

    pub fn start_server(tx: Sender<IpcCommand>) {
        let socket_path = Self::get_socket_path();
        let _ = std::fs::remove_file(&socket_path);

        thread::spawn(move || {
            let listener = match UnixListener::bind(&socket_path) {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("Failed to bind HyprOsk IPC socket at {:?}: {:?}", socket_path, e);
                    return;
                }
            };
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
                                "show-tiled" | "show tiled" | "show --tiled" | "tiled" => {
                                    ("OK: Shown Tiled\n", Some(IpcCommand::ShowMode(true)))
                                }
                                "show-floating" | "show floating" | "show --floating" | "show-overlay" | "show overlay" | "show --overlay" | "floating" | "overlay" => {
                                    ("OK: Shown Floating\n", Some(IpcCommand::ShowMode(false)))
                                }
                                "hide" => ("OK: Hidden\n", Some(IpcCommand::Hide)),
                                "toggle" => ("OK: Toggled\n", Some(IpcCommand::Toggle)),
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
    }
}
