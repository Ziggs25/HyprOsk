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
        self.width = effective_w;
        self.height = effective_h;
        self.is_configured = true;

        if self.is_visible {
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

    layer_surface.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer_surface.set_size(0, state.height);
    layer_surface.set_margin(0, state.config.general.margin_horizontal, state.config.general.margin_bottom, state.config.general.margin_horizontal);
    
    if state.config.general.exclusive_zone {
        layer_surface.set_exclusive_zone(state.height as i32);
    } else {
        layer_surface.set_exclusive_zone(0);
    }

    // CRITICAL: NEVER request keyboard interactivity on an OSK layer surface
    // This allows touch events while preserving keyboard focus on the client app!
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer_surface.commit();

    layer_surface
}
