# HyprOsk

Native Wayland on-screen keyboard for Hyprland. Written in Rust, runs as a layer-shell overlay and types via the Wayland virtual keyboard protocol.

It shows automatically when you focus a text field (GTK, Qt, Firefox, Chromium, Foot, etc.) and hides when focus is lost. No focus stealing — tapping keys does not move focus away from the app.

## What it does

- Auto show/hide on text input focus using `zwp_input_method_v2` + `zwp_text_input_v3`.
- Layer-shell window anchored to bottom (`zwlr_layer_shell_v1`, `keyboard_interactivity = 0`), touch/pointer input, output via `commit_string` / virtual keyboard.
- Layers: Lower, Upper (Shift), Symbols, Symbols2. Layout is Windows 11-style (Esc, Tab, Shift, Backspace, Ctrl/Alt/Super, arrows, Space, suggestion bar).
- Suggestion bar + clipboard history.
- Folio/tablet detection: can suppress auto-show when a physical keyboard is attached.

## Requirements

- Hyprland (or any wlroots compositor with `zwlr_layer_shell_v1` + `zwp_input_method_v2` + `zwp_virtual_keyboard_v1`)
- Wayland session
- For building from source: `cargo`, `pkg-config`, `wayland`, `wayland-protocols`, `libxkbcommon`, `wayland-scanner`

## Installation

### 1. Nix flake (recommended)

```bash
nix build .#default
./result/bin/hyprosk daemon

# or run directly without installing
nix run .# -- daemon
```

### 2. NixOS / Home Manager module

Flake provides `nixosModules.default` and `homeManagerModules.default`.

**NixOS (`flake.nix`):**
```nix
{
  inputs.hyprosk.url = "github:Ziggs25/HyprOsk";
  # ...
  outputs = { self, nixpkgs, hyprosk, ... }: {
    nixosConfigurations.yourhost = nixpkgs.lib.nixosSystem {
      modules = [
        hyprosk.nixosModules.default
        { programs.hyprosk.enable = true; }
      ];
    };
  }
}
```
This installs the package and enables a `systemd --user` service `hyprosk` bound to `graphical-session.target`.

**Home Manager:**
```nix
{
  inputs.hyprosk.url = "github:Ziggs25/HyprOsk";
  # ...
  homeManagerConfiguration = {
    imports = [ hyprosk.homeManagerModules.default ];
    programs.hyprosk = {
      enable = true;
      settings.general.height = 420;
      settings.behavior.folio_mode = true;
    };
  }
}
```
Config is written to `~/.config/hyprosk/config.toml`. Service is bound to `hyprland-session.target` by default.

Verify:
```bash
nix flake check
nix run .# -- --version
```

### 3. Cargo (any distro)

```bash
# install dependencies (example: Arch/Debian)
# arch: sudo pacman -S wayland wayland-protocols libxkbcommon pkgconf
# debian/ubuntu: sudo apt install libwayland-dev libxkbcommon-dev pkg-config

cargo build --release
./target/release/hyprosk daemon
```

## Hyprland setup

Add to `~/.config/hypr/hyprland.conf`:

```ini
exec-once = hyprosk daemon

# optional manual toggle
bind = SUPER, K, exec, hyprosk toggle

layerrule = blur, hyprosk
layerrule = ignorezero, hyprosk
layerrule = animation slide bottom, hyprosk
```

Optional swipe gesture (with hypr-touch):
```ini
plugin:touch {
    gesture = 1, up, edge, bottom, exec, hyprosk toggle
}
```

## Usage

Daemon must be running. Default command with no args also starts the daemon:

```bash
hyprosk              # same as hyprosk daemon
hyprosk daemon       # start daemon (foreground)
hyprosk show         # show
hyprosk hide         # hide
hyprosk toggle       # toggle
hyprosk layer lower      # switch to lower case
hyprosk layer upper      # switch to upper case
hyprosk layer symbols    # symbols page 1
hyprosk layer symbols2   # symbols page 2
hyprosk clipboard    # toggle clipboard history view
hyprosk status       # folio/tablet/keyboard detection + current auto-show state
hyprosk quit         # stop daemon
```

Custom config path:
```bash
hyprosk --config /path/to/config.toml daemon
```

## Configuration

File: `~/.config/hyprosk/config.toml` (auto-created on first run with defaults).

```toml
[general]
height = 420              # keyboard height in px
margin_bottom = 0
margin_horizontal = 0
corner_radius = 0.0
exclusive_zone = true     # true = push tiled windows up
theme_name = "catppuccin" # theme name (currently wireframe palette is fixed in ui/osk.slint)

[behavior]
auto_show = true
auto_hide = true
hide_on_fullscreen = true
folio_mode = false         # if true, suppress auto-show when physical keyboard attached
touch_only = false         # if true, auto-show only when focus was triggered by touch
long_press_ms = 400
repeat_delay_ms = 350
repeat_rate_ms = 45
feedback_command = ""     # optional: e.g. "paplay /usr/share/sounds/click.ogg"

[theme]
background = "#1e1e2ecc"
key_background = "#313244"
key_pressed = "#89b4fa"
key_special = "#45475a"
text_color = "#cdd6f4"
text_special = "#f5e0dc"
accent_color = "#cba6f7"
border_color = "#585b7066"
border_width = 1.0
key_radius = 8.0
key_spacing = 6.0
opacity = 0.95
```

> Note: Rendering uses Slint as a headless software renderer into the Wayland SHM buffer. The colors in `ui/osk.slint` are currently the source of truth for the on-screen palette. `theme` values are kept for compatibility/fallback.

## How auto-show works

```
App (GTK/Qt/Firefox) -- zwp_text_input_v3 --> Compositor (Hyprland) -- zwp_input_method_v2 (activate/deactivate) --> HyprOsk -- zwp_virtual_keyboard_v1 (commit_string) --> App
```

HyprOsk is a layer-shell surface at the bottom. It never takes keyboard focus.

## App compatibility

Some apps need env vars/flags to announce text input over Wayland:

- **GTK 3:** `GTK_IM_MODULE=wayland`
- **Qt 5/6:** `QT_IM_MODULE=wayland` + `QT_QPA_PLATFORM="wayland;xcb"`
- **Chromium / Electron / VS Code:** `--ozone-platform-hint=auto --ozone-platform=wayland --enable-wayland-ime`
- **Firefox:** `MOZ_ENABLE_WAYLAND=1`
- **GTK 4 / Foot:** works without extra config

If auto-show does not trigger, run `hyprosk status` and check your compositor supports `zwp_input_method_v2`.

## License

MIT OR Apache-2.0
