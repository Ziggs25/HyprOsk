use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;

#[derive(Debug, Clone)]
pub enum HyprlandEvent {
    Fullscreen(bool),
    ActiveWindow { class: String, title: String },
    WorkspaceChanged(String),
}

pub struct HyprlandIpcListener;

impl HyprlandIpcListener {
    pub fn get_socket2_path() -> Option<PathBuf> {
        let xdg_runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
        let his = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        let path = PathBuf::from(xdg_runtime)
            .join("hypr")
            .join(his)
            .join(".socket2.sock");
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    pub fn start_listener(tx: Sender<HyprlandEvent>) {
        thread::spawn(move || {
            let socket_path = match Self::get_socket2_path() {
                Some(p) => p,
                None => {
                    tracing::warn!("Hyprland socket2.sock not found. Hyprland IPC listener disabled.");
                    return;
                }
            };

            tracing::info!("Connecting to Hyprland IPC socket at {:?}", socket_path);
            let stream = match UnixStream::connect(&socket_path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to connect to Hyprland socket2: {:?}", e);
                    return;
                }
            };

            let reader = BufReader::new(stream);
            for line_res in reader.lines() {
                let line = match line_res {
                    Ok(l) => l,
                    Err(_) => break,
                };

                if let Some((event_type, event_data)) = line.split_once(">>") {
                    match event_type {
                        "fullscreen" => {
                            let is_fullscreen = event_data.trim() == "1";
                            let _ = tx.send(HyprlandEvent::Fullscreen(is_fullscreen));
                        }
                        "activewindow" => {
                            let (class, title) = event_data.split_once(',').unwrap_or((event_data, ""));
                            let _ = tx.send(HyprlandEvent::ActiveWindow {
                                class: class.to_string(),
                                title: title.to_string(),
                            });
                        }
                        "workspace" => {
                            let _ = tx.send(HyprlandEvent::WorkspaceChanged(event_data.to_string()));
                        }
                        _ => {}
                    }
                }
            }
        });
    }
}
