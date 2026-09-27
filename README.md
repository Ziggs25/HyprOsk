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

- Hyprland (or any compositor with `zwlr_layer_shell_v1` + `zwp_virtual_keyboard_v1`; `zwp_input_method_v2` is needed for readable surrounding text and automatic text-field activation)
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
surrounding_text_readback = false # default false: avoids buggy browser/host text readback
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

## Text exchange and app coverage

HyprOsk detects and reports the text protocols exposed by the compositor at
startup. Its text insertion path is `zwp_input_method_v2`: when the compositor and
focused app support it, HyprOsk commits text cleanly using `commit_string`.

By default, host surrounding-text readback is disabled (`surrounding_text_readback = false`
in `config.toml`) to prevent buggy browser (e.g. Helium/Chromium) readback and focus races
from corrupting the OSK suggestion state. In this mode, HyprOsk continues to commit text
cleanly over `zwp_input_method_v2` and fall back to `zwp_virtual_keyboard_v1`, without
reading host text snapshots. Users who want host text readback can explicitly opt in with
`surrounding_text_readback = true`.

The `zwp_text_input_v3` global is also probed for diagnostics. It is the
application-to-compositor side of the Wayland text-input handshake; merely
binding its manager does not let an unrelated client read another app's text.

When input-method-v2 is unavailable, HyprOsk uses
`zwp_virtual_keyboard_v1`. That fallback can type into most apps, terminals,
and XWayland windows, but it is write-only: the daemon cannot inspect existing
text or selections. XWayland apps may get better readback through their own
GTK/Qt/XIM, IBus, or Fcitx integration, depending on how the app was built.

There is no safe Wayland API for universally scraping every app's text. Apps
that do not expose a text-input or accessibility integration must use the
write-only keyboard path.

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

If auto-show does not trigger, run `hyprosk status` and inspect the daemon log
for the reported text-input capabilities. A compositor may advertise the
protocol globals while a particular seat still does not activate an input
method; those apps will use the virtual-keyboard fallback.

## Testing

The repeatable automated and live-seat validation steps are documented in
[TESTING.md](TESTING.md). Run the automated checks before testing text-field
behavior manually. HyprOsk allows one daemon per seat and rejects duplicate
launches before they can compete for `zwp_input_method_v2`.

## License

MIT OR Apache-2.0
