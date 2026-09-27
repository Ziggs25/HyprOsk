use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use wayland_client::{Connection, QueueHandle};
use crate::wayland::state::WaylandState;

impl LayerShellHandler for WaylandState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        tracing::info!("Layer surface closed by compositor");
        self.is_running = false;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (w, h) = configure.new_size;
        let effective_w = if w > 0 { w } else { self.width };
        let effective_h = if h > 0 { h } else { self.height };

        tracing::info!("LayerSurface configured: {}x{}", effective_w, effective_h);
        let size_changed = self.width != effective_w || self.height != effective_h;
        self.width = effective_w;
        self.height = effective_h;
        self.is_configured = true;
        if size_changed {
            self.sync_layout_and_redraw(qh);
        } else if self.is_visible {
            self.redraw(qh);
        }
    }
}

pub fn create_layer_surface(
    state: &mut WaylandState,
    qh: &QueueHandle<WaylandState>,
) -> LayerSurface {
    let surface = state.compositor_state.create_surface(qh);
    let layer_surface = state.layer_shell.create_layer_surface(
        qh,
        surface,
        Layer::Overlay,
        Some("hyprosk"),
        None,
    );

    let target_h = state.effective_keyboard_height();
    let (target_w, margin_b, margin_h) = if let Some(ref custom) = state.custom_layout {
        let w = custom.width.unwrap_or(0);
        let mb = custom.margin_bottom.unwrap_or(state.config.general.margin_bottom);
        let mh = if custom.width.is_some() { 0 } else { state.config.general.margin_horizontal };
        (w, mb, mh)
    } else {
        (0, state.config.general.margin_bottom, state.config.general.margin_horizontal)
    };

    state.height = target_h;
    if target_w > 0 {
        layer_surface.set_anchor(Anchor::BOTTOM);
        layer_surface.set_size(target_w, target_h);
        layer_surface.set_margin(0, 0, margin_b, 0);
    } else {
        layer_surface.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer_surface.set_size(0, target_h);
        layer_surface.set_margin(0, margin_h, margin_b, margin_h);
    }
    
    let zone = state.active_exclusive_zone.unwrap_or(if state.config.general.exclusive_zone {
        state.height as i32
    } else {
        0
    });
    layer_surface.set_exclusive_zone(zone);

    // CRITICAL: NEVER request keyboard interactivity on an OSK layer surface
    // This allows touch events while preserving keyboard focus on the client app!
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer_surface.commit();

    layer_surface
}
