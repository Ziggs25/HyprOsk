pub mod config;
pub mod folio;
pub mod hyprland;
pub mod ipc;
pub mod layout;
pub mod render;
pub mod suggest;
pub mod touch;
pub mod wayland;

use anyhow::{Context, Result};
use smithay_client_toolkit::{
    compositor::CompositorState, output::OutputState, registry::RegistryState, seat::SeatState,
    shell::wlr_layer::LayerShell, shm::Shm,
};
use std::os::fd::{AsFd, AsRawFd};
use std::path::Path;
use std::sync::mpsc;
use wayland_client::Connection;
use wayland_client::globals::registry_queue_init;
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;

use crate::config::Config;
use crate::hyprland::{HyprlandEvent, HyprlandIpcListener};
use crate::ipc::{IpcCommand, IpcServer};
use crate::layout::LayerId;
use crate::wayland::WaylandState;

pub fn run_daemon(config_path: Option<&Path>) -> Result<()> {
    tracing::info!("Starting HyprOsk (HeliBoard-inspired Wayland On-Screen Keyboard Daemon)...");
    let config = Config::load_or_create(config_path);

    // Claim the IPC endpoint before binding any Wayland input-method object.
    // This keeps duplicate daemon launches from competing for the one
    // input-method-v2 object allowed on a seat.
    let (ipc_tx, ipc_rx) = mpsc::channel();
    IpcServer::start_server(ipc_tx).context("another HyprOsk daemon may already be running")?;

    let conn = Connection::connect_to_env().context("Failed to connect to Wayland display")?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).context("Failed to initialize Wayland registry queue")?;
    let qh = event_queue.handle();

    let compositor_state =
        CompositorState::bind(&globals, &qh).context("wl_compositor global missing")?;
    let output_state = OutputState::new(&globals, &qh);
    let shm_state = Shm::bind(&globals, &qh).context("wl_shm global missing")?;
    let seat_state = SeatState::new(&globals, &qh);
    let layer_shell = LayerShell::bind(&globals, &qh).context(
        "zwlr_layer_shell_v1 missing: compositor must support layer-shell (e.g. Hyprland)",
    )?;

    // Attempt to bind zwp_input_method_manager_v2 for automatic input field detection
    let im_manager: Option<ZwpInputMethodManagerV2> = globals.bind(&qh, 1..=1, ()).ok();
    if im_manager.is_some() {
        tracing::info!(
            "Successfully bound zwp_input_method_manager_v2 for automatic text field detection!"
        );
    } else {
        tracing::warn!(
            "zwp_input_method_manager_v2 not found. Auto-show on text focus requires input-method-v2 support."
        );
    }

    // This is an app-to-compositor protocol. Probe it for diagnostics, but do
    // not treat it as permission to read another client's text; input-method
    // v2 remains the daemon's readable text-context path.
    let text_input_v3_manager: Option<ZwpTextInputManagerV3> = globals.bind(&qh, 1..=1, ()).ok();
    if text_input_v3_manager.is_some() {
        tracing::info!("Compositor advertises zwp_text_input_v3");
    } else {
        tracing::warn!("Compositor does not advertise zwp_text_input_v3");
    }

    let vk_manager: Option<ZwpVirtualKeyboardManagerV1> = globals.bind(&qh, 1..=1, ()).ok();

    let registry_state = RegistryState::new(&globals);
    let shm_ref = Shm::bind(&globals, &qh).context("Rebinding shm")?;

    let mut state = WaylandState::new(
        registry_state,
        compositor_state,
        output_state,
        shm_state,
        seat_state,
        layer_shell,
        &shm_ref,
        config.clone(),
    );

    state.im_manager = im_manager.clone();
    state.vk_manager = vk_manager;
    state.text_input_capabilities.input_method_v2 = im_manager.is_some();
    state.text_input_capabilities.text_input_v3 = text_input_v3_manager.is_some();
    state.text_input_capabilities.virtual_keyboard_v1 = state.vk_manager.is_some();
    drop(text_input_v3_manager);

    // Initial roundtrip to populate seats and outputs
    event_queue
        .roundtrip(&mut state)
        .context("Initial Wayland roundtrip failed")?;

    // Bind input method to default seat if manager exists
    if let Some(ref imm) = state.im_manager
        && let Some(seat) = state.seat_state.seats().next()
    {
        let im = imm.get_input_method(&seat, &qh, ());
        state.input_method = Some(im);
        tracing::info!("Registered zwp_input_method_v2 on seat {:?}", seat);
    }

    // Bind virtual keyboard for keycodes (arrows, Esc, and fallback keys)
    state.init_virtual_keyboard(&qh);
    state.log_text_input_capabilities();

    // Set up the remaining event channels. The IPC channel was created before
    // Wayland setup so duplicate daemons fail before claiming the seat.
    let (hypr_tx, hypr_rx) = mpsc::channel();
    let (folio_tx, folio_rx) = mpsc::channel();
    let (touch_tx, touch_rx) = mpsc::channel();

    HyprlandIpcListener::start_listener(hypr_tx);

    if config.behavior.folio_mode {
        crate::folio::spawn_folio_poller(folio_tx);
        tracing::info!(
            "Folio mode enabled: auto-show suppressed while a physical keyboard is attached (current: {})",
            state.folio_attached
        );
    }
    if config.behavior.touch_only {
        tracing::info!(
            "touch_only enabled: automatic input-method activation requires recent touchscreen input"
        );
    }
    if config.behavior.auto_show || config.behavior.touch_only {
        crate::touch::spawn_touch_poller(touch_tx);
        tracing::info!(
            "Touchscreen monitoring enabled for input source tracking and touch_only gating"
        );
    }

    tracing::info!("HyprOsk daemon successfully initialized and running.");

    while state.is_running {
        // Dispatch pending Wayland events
        event_queue.dispatch_pending(&mut state)?;

        // Check external IPC commands (CLI hyprosk show / hide / toggle)
        while let Ok(cmd) = ipc_rx.try_recv() {
            match cmd {
                IpcCommand::Show => state.show_keyboard(&qh),
                IpcCommand::ShowMode(exclusive) => {
                    state.show_keyboard_with_exclusivity(&qh, exclusive)
                }
                IpcCommand::ShowWithMode(mode) => {
                    state.show_keyboard_with_mode(&qh, mode);
                }
                IpcCommand::Hide => state.hide_keyboard(&qh),
                IpcCommand::Toggle => state.toggle_keyboard(&qh),
                IpcCommand::ToggleMode(exclusive) => {
                    state.toggle_keyboard_with_exclusivity(&qh, exclusive)
                }
                IpcCommand::ToggleWithMode(mode) => {
                    state.toggle_keyboard_with_mode(&qh, mode);
                }
                IpcCommand::ToggleExclusivity => state.toggle_exclusivity(&qh),
                IpcCommand::SetExclusivity(exclusive) => state.set_exclusivity(&qh, exclusive),
                IpcCommand::SetLayoutMode(mode) => state.set_layout_mode(mode, &qh),
                IpcCommand::Reload => state.reload_config(&qh),
                IpcCommand::LoadLayout(name) => {
                    if let Err(e) = state.load_layout(&name, &qh) {
                        tracing::error!("Failed to load layout {}: {:?}", name, e);
                    }
                }
                IpcCommand::Quit => state.is_running = false,
                IpcCommand::SwitchLayer(layer_name) => {
                    let layer = match layer_name.to_lowercase().as_str() {
                        "upper" | "shift" | "caps" => LayerId::Upper,
                        "symbols" | "sym" | "123" | "num" | "numbers" => LayerId::Symbols,
                        "symbols2" | "sym2" => LayerId::Symbols2,
                        _ => LayerId::Lower,
                    };
                    state.switch_layer(layer, &qh);
                }
                IpcCommand::Clipboard => {
                    state.toggle_clipboard(&qh);
                }
            }
        }

        // Check Hyprland IPC events
        while let Ok(event) = hypr_rx.try_recv() {
            match event {
                HyprlandEvent::Fullscreen(is_fs) => {
                    if is_fs && state.config.behavior.hide_on_fullscreen {
                        state.hide_keyboard(&qh);
                    }
                }
                HyprlandEvent::ActiveWindow { class, title } => {
                    tracing::debug!("Active window changed: {} ({})", class, title);
                }
                HyprlandEvent::WorkspaceChanged(_) => {
                    state.note_workspace_switch();
                    if state.is_visible && !state.manually_shown {
                        state.hide_keyboard(&qh);
                    }
                }
            }
        }

        // Check folio attach/detach changes
        while let Ok(attached) = folio_rx.try_recv() {
            state.on_folio_change(attached, &qh);
        }
        while let Ok(event) = touch_rx.try_recv() {
            state.note_touch(event);
        }

        // Update hold-preview for long-press secondary visualization (Gboard/HeliBoard style)
        state.update_hold_preview(&qh);
        // Handle key repeat (e.g. hold backspace to continuously delete)
        state.update_key_repeat(&qh);
        #[cfg(feature = "futo-swipe")]
        state.update_swipe_preview(&qh);
        state.update_swipe_trail(&qh);

        // Flush and wait for next event
        conn.flush()?;
        if let Some(guard) = event_queue.prepare_read() {
            let mut poll_fd = libc::pollfd {
                fd: conn.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe {
                libc::poll(
                    &mut poll_fd,
                    1,
                    if state.has_frame_activity() { 16 } else { 32 },
                )
            };
            if ready < 0 {
                return Err(std::io::Error::last_os_error()).context("polling Wayland connection");
            }
            if ready > 0 {
                let failure = poll_fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL);
                if failure != 0 {
                    anyhow::bail!("Wayland connection poll failed (revents={failure:#x})");
                }
                if poll_fd.revents & libc::POLLIN != 0 {
                    guard.read().context("reading Wayland connection")?;
                }
            }
        }
    }

    tracing::info!("HyprOsk daemon shutting down gracefully.");
    Ok(())
}
