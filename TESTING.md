# HyprOsk testing playbook

Run the automated checks from the repository root:

```sh
nix-shell shell.nix --run 'cargo test -q'
nix-shell shell.nix --run 'cargo build'
nix-shell shell.nix --run 'cargo build --release'
```

## Input-method-v2 and duplicate-daemon check

Hyprland permits one `zwp_input_method_v2` object per seat. HyprOsk claims
its IPC socket before connecting to Wayland so a second daemon exits cleanly
instead of creating an unusable input-method object.

1. Stop any old HyprOsk processes and start one daemon.
2. Confirm that exactly one daemon is running:

   ```sh
   pgrep -af 'hyprosk.*daemon|target/(debug|release)/hyprosk'
   ```

3. Confirm the compositor advertises the relevant globals:

   ```sh
   nix shell nixpkgs#wayland-utils --command wayland-info \
     | rg 'zwp_input_method_manager_v2|zwp_text_input_manager_v3|zwp_virtual_keyboard_manager_v1'
   ```

4. Start a second daemon. It must exit with an IPC message containing
   `another HyprOsk daemon is already running`, and it must not claim a second
   Wayland input-method object.
5. Focus a GTK, Qt, Chromium/Helium, and Firefox text field. The daemon log
   should report `input-method-v2 (write-only, readback disabled)` by default
   (or `input-method-v2 (read/write)` if `surrounding_text_readback = true`)
   while the field is active and should return to the virtual-keyboard
   fallback after deactivation.
6. Exercise the browser inline-completion case (`w` → `www.youtube.com`). Tap
   an OSK candidate such as `we` or `well`; the field must become exactly
   `we` or `well`, never `wwe` or `wwell`. Repeat after deleting characters.
7. Exercise OSK ArrowUp/ArrowDown candidate clearing: type a prefix (e.g. `hel`)
   so suggestion candidates appear on the OSK bar. Tap or hold OSK `ArrowDown` or
   `ArrowUp`; candidates must immediately clear and stay cleared across key
   repeats, preventing misleading stale autocomplete suggestions. Then tap a
   fresh letter key (e.g. `p`); candidates must regenerate cleanly from new typing.
8. Exercise surrounding-text readback: verify that by default
   (`surrounding_text_readback = false`), OSK typing and suggestion selection
   still commit text smoothly via `zwp_input_method_v2` without reading host
   surrounding-text snapshots. When explicitly enabled (`surrounding_text_readback = true`),
   surrounding text is read only when supported by the host.

## Cursor-position auto-placement check

On `input-method-v2` auto-show (`Activate`), HyprOsk samples Hyprland's global cursor
coordinates via Hyprland IPC as a best-effort touch proxy to infer whether the active
input field is located in the upper or lower half of the monitor:

- **Upper half (`rel_y < 0.5`)**: HyprOsk displays as a **floating overlay** (`exclusive_zone = 0`)
  and requests no reserved area. Verify the actual window behavior on your Hyprland layout.
- **Lower half (`rel_y >= 0.5`)**: HyprOsk displays as **docked** (`exclusive_zone = effective_keyboard_height`).
  HyprOsk requests space reservation from the compositor via layer-shell; actual window adjustment
  or shifting is compositor- and layout-dependent (verify how Hyprland adjusts your specific active
  workspace and tiled/floating window layout).

### Manual Verification Procedure

1. **Top-half touch / click**:
   - Tap or click into an input field located in the upper half of your monitor (such as a browser URL/search bar near the top edge).
   - Verify HyprOsk appears as a floating overlay; observe whether the underlying window moves.
   - Check the daemon log:
     ```
     Auto-placement best-effort inference: cursor=(X, Y) on monitor '...' (WxH at X,Y), rel_y=0.XX -> upper half -> floating overlay (exclusive_zone=0)
     ```
2. **Bottom-half touch / click**:
   - Tap or click into an input field located in the lower half of your monitor (such as a comment box or bottom terminal prompt).
   - Verify HyprOsk requests docked space reservation (`exclusive_zone = effective_keyboard_height`), and verify actual Hyprland window layout adjustment.
   - Check the daemon log:
     ```
     Auto-placement best-effort inference: cursor=(X, Y) on monitor '...' (WxH at X,Y), rel_y=0.XX -> lower half -> docked (exclusive_zone=height)
     ```
3. **Compare with `hyprctl cursorpos`**:
   - Run `hyprctl cursorpos` in a terminal. Verify whether `cursorpos` follows touch on your setup (behavior can vary across touchscreen drivers and Hyprland versions/configurations) and verify that the logged cursor coordinates match `hyprctl cursorpos`.
4. **Manual invocations**:
   - Run `hyprosk show` or `hyprosk toggle` via CLI or shortcut.
   - Verify that manual show respects the user-configured `exclusive_zone` in `config.toml` directly, without cursor inference override.
5. **Programmatic / Keyboard focus caveat**:
   - Verify whether `cursorpos` follows touch on your specific setup. While direct touchscreen taps and mouse clicks generally update cursor coordinates, programmatic focus shifts (e.g. `Tab` key navigation or auto-focusing popups) do not move Hyprland's hardware cursor to the newly focused field. Under programmatic focus, cursor position reflects the last touch/pointer position.
   - If IPC queries time out, fail (e.g. non-Hyprland compositors), or yield non-finite coordinates / coordinates outside all active monitor rectangles, HyprOsk logs an informational notice (`Auto-placement: best-effort cursor inference unavailable; falling back to configured exclusive_zone=...`) and falls back safely to the configured `exclusive_zone`.

If a daemon was killed abruptly, start the next one normally. HyprOsk detects
and reclaims the stale IPC socket.

## Comment style

Keep source comments short and concrete. Explain one reason or invariant; do
not narrate code that is already clear.
