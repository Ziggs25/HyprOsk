# HyprOsk — Slint Overhaul Roadmap

> Authoritative companion to `keyboard wireframe.md`.
> Kept in-repo so any agent (opencode/cline) can resume instantly.

## Goal

Overhaul HyprOsk's on-screen keyboard to the Windows-11-style wireframe look,
using **Slint purely as a headless paint component** for the existing
Rust-owned Wayland/input stack (typing only). Packaging becomes
NixOS-first / Home-Manager-compatible.

## Constraints

- Slint = paint bucket ONLY: `default-features = false`, no winit/windowing;
  renders into the existing `wl_shm` slot.
- `keyboard wireframe.md` is the source of truth (look AND layout, incl. SVG
  icon paths — no online icon lookup).
- Only 2 layout layers: **Letters ↔ Symbols** (Numbers/Nav/Emoji dropped);
  Shift toggles `Lower ↔ Upper` within Letters.
- `Win` = Super (evdev 125). `Mic` renders visually with **no action**.
- Typing behavior stays; gesture/IM/VK identical; zero-warning Rust.
- NixOS first: flake `package` output + Home Manager module (packages, config
  file, systemd user service, desktop entry), verified via `nix flake check`
  + `nix run`.

## Milestones

- [x] **M1 — Spike** (opencode, done): slint-only crate, headless Platform +
      WindowAdapter + render into buffer, glyphs (⌫⏎⇧⊞◀▶▲▼🎤) + SVG, PNG
      eyeball. Repo: `/tmp/opencode/slint-spike` (watch: `/tmp` may be wiped).
- [x] **M2 — Wireframe layout restructure** (cline, done):
      - `src/layout/key.rs`: `LayerId` → `Lower/Upper/Symbols`; new
        `KeyAction::{Ctrl, Alt, Win, Home, End, None}`.
      - `src/layout/layouts.rs`: rewritten to wireframe rows/metrics:
        suggestion bar + `Esc 1.1, Tab 1.4, Shift/Enter 1.8, Backspace 1.6,
        toggle 1.25, Ctrl/Alt/Win/Mic/arrow 1.1, Space 7.2`; dual digit
        sub-labels on qwertyuiop; symbols Layout B (◀▶ · ; : ( ) / ' " ?
        Home ▲ End).
      - `src/lib.rs`: IPC `layer` names → upper/symbols only.
      - `src/wayland/state.rs`: `adapt_layout_for_content_purpose` → Symbols;
        handled `Ctrl/Alt/Win/Home/End/None` (momentary evdev taps:
        ctrl=29, alt=56, meta=125, home=102, end=107); `send_keycode` reused.
      - Verified: `cargo check` clean; **zero new** clippy warnings; release
        build OK.
- [x] **3 — Bridge** (cline, done): `ui/osk.slint` full scene + `src/render/slint.rs`
      + `build.rs` + pinned slint deps in `Cargo.toml` (`=1.14.0`,
      `renderer-software`, `i-slint-core` with `svg`, `slint-build`).
      Vendored `ui/fonts/DejaVuSans*.ttf`. `RenderEngine::calculate_key_rects`
      still the geometry source of truth.
- [x] **4 — Paint swap** (cline, done): `state.rs::redraw()` lazily creates a
      `SlintScene` and renders the wireframe into the existing SHM ARGB8888
      slot (B,G,R,A bytes from Slint's premultiplied R,G,B,A); legacy
      `RenderEngine` is the automatic fallback if Slint init/render fails.
- [x] **5 — Wiring** (cline, done): `SlintScene` self-manages the platform
      (created on first redraw), ticks `update_timers_and_animations()` per
      frame; config default `height` → 420.
- [x] **6 — Hygiene** (opencode, verify): `cargo check` (including example) clean,
      **zero new** clippy warnings (total 15 < baseline 18; agent's 2 new
      `collapsible_if` from the Paste/send_text restructure collapsed via
      let-chains).
- [x] **7 — Visual QA** (opencode, verify): `examples/preview.rs` renders the real
      QWERTY scene (3 suggestions) headlessly to `preview.png`; all 4 probes
      PASS. **Fix applied**: `SlintScene::apply_theme` was overriding the
      wireframe palette with legacy catppuccin colors — removed; palette is now
      fixed in `ui/osk.slint` (dock #1a1a1a, key #2d2d2d, pressed #1e1e1e, edge
      #222, sub #c8c8c8). `RenderEngine::calculate_key_rects` now uses wireframe
      metrics (padding 8, gap 8, suggestion bar 44 → 80px keys at 420 height).
      Run with `nix-shell shell.nix --run 'cargo run --example preview'`.
- [x] **8 — NixOS packaging** (opencode, verify): `nix flake check` passes;
      `nix build .#default` succeeds (Slint deps + vendored fonts build in
      sandbox; note: untracked files must be `git add`-ed before flake builds
      since nix sources are git-tracked); `nix run .# -- --version` → 0.1.0;
      HM module evaluated with real `homeManagerConfiguration`: service
      ExecStart, `hyprland-session.target` WantedBy and generated
      `~/.config/hyprosk/config.toml` all correct.

## Slint 1.14.0 API gotchas (learned during spike)

- `slint::platform::set_platform(Box::new(Sp))` must run **before**
  `OskUi::new()`.
- `Platform::create_window_adapter()` → `Result<Rc<dyn WindowAdapter>,
  PlatformError>`; `Rc::<Adapter>::new_cyclic(|weak| Adapter {
  window: Rc::new(Window::new(weak.clone())), ... })` — `Weak<Rc<Adapter>>`
  coerces to `Weak<dyn WindowAdapter>` automatically.
- `WindowEvent::Resized { size: LogicalSize }` (not Physical); render into
  `PremultipliedRgbaColor` (fields red/green/blue/alpha, premultiplied =
  ready for `wl_shm` ARGB) via `SoftwareRenderer::render(&mut buf, w)`.
- Font pipeline triple-lock: `build.rs` `embed_resources(
  EmbedForSoftwareRenderer)` + `import "fonts/DejaVuSans.ttf";` in the
  `.slint` + exact family name `"DejaVu Sans"` in `default-font-family`.
  Panic to expect otherwise: *"EmbedForSoftwareRenderer option"*.
- SVG icons at runtime via `Image::load_from_svg_data` with `{COLOR}`
  templating — then assign to `<image>` properties. Do **not** use
  `@image-url` with missing files at compile time.
- No grouped border-radius: use `border-top-left-radius` etc. Window
  `background: transparent` gives true alpha-0 corners.
- `i-slint-core` direct dep must be `=1.14.0` for feature unification;
  `slint-build::compile_with_config` is the entry point.
- Toolchain on this machine: only via
  `nix-shell shell.nix --run 'cargo ...'`
  (no system-wide cargo). `/tmp` can be wiped between agent sessions.

## Verification commands

```bash
cd HyprOsk
nix-shell shell.nix --run 'cargo check'
nix-shell shell.nix --run 'cargo clippy --all-targets'
nix-shell shell.nix --run 'cargo build --release'
```