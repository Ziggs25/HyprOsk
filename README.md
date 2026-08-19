# HyprOsk 🚀
> Fast, lightweight, native Wayland On-Screen Keyboard designed for Hyprland with automatic input-field detection.

---

## 🌟 Overview

**HyprOsk** is a blazing-fast, minimal on-screen keyboard engineered specifically for **Hyprland** and **wlroots** Wayland compositors in Rust.

### Key Highlights
- **⚡ Instant Startup & Nano Footprint:** Written in modern Rust with zero heavy web/Qt runtimes (~6–10 MB RAM RSS, < 5ms startup).
- **🎯 Automatic Input-Field Detection:** Automatically pops up when you tap into a text field in GTK, Qt, Firefox, Foot, or Chromium, and hides when focus is lost (powered by `zwp_input_method_v2` and `zwp_text_input_v3`).
- **🛡️ Zero-Focus-Stealing Architecture:** Uses `zwlr_layer_shell_v1` with `keyboard_interactivity = 0` so tapping keys never breaks cursor focus in your active application.
- **✨ Multiple Layers:** QWERTY Lowercase, Uppercase, Numbers, Extended Math & Symbols, Navigation/Edit Mode (Arrows, Copy, Paste, Tab, Esc), and Emoji panel.
- **🎨 Built-in Aesthetic Themes:** Catppuccin Mocha, Tokyo Night, OLED Dark, with custom rounding, borders, margins, and transparency.
- **🔌 Hyprland IPC & CLI Control:** Real-time IPC server for gestures, keybindings, and automatic hiding on fullscreen.

---

## 🏗️ Architecture & How Auto-Detection Works

HyprOsk implements the full Wayland Input Method lifecycle:

```
+-----------------------------------------------------------------------------------+
|                           Target Application (e.g. GTK4 / Firefox)                |
+-----------------------------------------------------------------------------------+
                               |  zwp_text_input_v3
                               |  - enable() / disable()
                               |  - set_surrounding_text()
                               |  - set_content_type(hint, purpose)
                               v
+-----------------------------------------------------------------------------------+
|                                Compositor (Hyprland)                              |
+-----------------------------------------------------------------------------------+
                               |  zwp_input_method_v2
                               |  - activate()   ---> triggers HyprOsk to show
                               |  - deactivate() ---> triggers HyprOsk to hide
                               |  - content_type() -> auto switches to number pad for PINs
                               v
+-----------------------------------------------------------------------------------+
|                           HyprOsk (Wayland OSK Daemon)                            |
|                                                                                   |
|  1. Layer:      zwlr_layer_shell_v1 (Anchor: BOTTOM, keyboard_interactivity: 0)  |
|  2. Input:      wl_touch / wl_pointer events                                      |
|  3. Output:     zwp_input_method_v2.commit_string(utf8)                           |
+-----------------------------------------------------------------------------------+
```

---

## 📦 Building & Running

### Using Nix / NixOS
```bash
# Enter nix development shell
nix-shell

# Build release binary
cargo build --release

# Run HyprOsk daemon
./target/release/hyprosk daemon
```

### Standard Linux (Cargo)
Ensure `libwayland`, `libxkbcommon`, and `pkg-config` are installed:
```bash
cargo build --release
```

---

## ⚙️ Hyprland Integration (`hyprland.conf`)

Add the following to your `~/.config/hypr/hyprland.conf`:

```ini
# Launch HyprOsk on startup
exec-once = hyprosk daemon

# Optional: Bind a manual toggle key (e.g., Super + K)
bind = $mainMod, K, exec, hyprosk toggle

# Layer rules for smooth animations and blur
layerrule = blur, hyprosk
layerrule = ignorezero, hyprosk
layerrule = animation slide bottom, hyprosk
```

### Touch Gestures (e.g., with `hyprgrass`)
To swipe up from the bottom edge to show the keyboard:
```ini
plugin:hyprgrass {
    gesture = 1, up, edge, bottom, exec, hyprosk toggle
}
```

---

## 🎮 CLI Controls

```bash
hyprosk show          # Show the keyboard
hyprosk hide          # Hide the keyboard
hyprosk toggle        # Toggle visibility
hyprosk layer upper   # Switch to upper case
hyprosk layer num     # Switch to numbers
hyprosk layer sym     # Switch to symbols
hyprosk layer nav     # Switch to navigation keys
hyprosk layer emoji   # Switch to emoji picker
hyprosk quit          # Stop daemon
```

---

## 🎨 Configuration (`~/.config/hyprosk/config.toml`)

```toml
[general]
height = 320
margin_bottom = 12
margin_horizontal = 16
corner_radius = 18.0
exclusive_zone = true   # Pushes tiled windows up when open
theme_name = "catppuccin"

[behavior]
auto_show = true        # Automatically show when tapping input fields
hide_on_fullscreen = true
long_press_ms = 400

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
```

---

## 💡 Application Compatibility Matrix

To ensure all applications notify Hyprland when focusing text fields:

- **GTK 4 / Foot Terminal:** Works natively out of the box.
- **GTK 3:** Set `GTK_IM_MODULE=wayland` in your environment.
- **Qt 5 / Qt 6:** Set `QT_IM_MODULE=wayland` and `QT_QPA_PLATFORM="wayland;xcb"`.
- **Chromium / Electron / VSCode:** Launch with:
  ```bash
  --ozone-platform-hint=auto --ozone-platform=wayland --enable-wayland-ime
  ```
- **Firefox:** Enable Wayland windowing (`MOZ_ENABLE_WAYLAND=1`).
