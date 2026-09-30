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

## Folio-mode detection check

With `behavior.folio_mode = true`, run `hyprosk status` while the folio is
connected. Expect `folio USB (044e:1218): true`, `folio attached (verdict): true`,
`folio source: usb-044e:1218`, and `auto-show will be: SUPPRESSED`.

Source precedence is deliberate:

1. `044e:1218` (Alps Touchpad) present -> attached.
2. letter-capable evdev / `/proc` keyboard present -> attached.
3. `SW_TABLET_MODE` alone -> detached.

`SW_TABLET_MODE` is never allowed to *enable* suppression: on this Dell it comes
from `intel-hid` (INT33D5:00, "Intel HID switches") which only initializes the
switch on the first `0xcc`/`0xcd` ACPI notify, and the value can latch across
s2idle/hibernate. It is therefore only trusted when no keyboard was observed at
all, so a flaky switch can never leave the user without any keyboard.

### Why the folio cannot be detected as "folded"

On the Latitude 7200 2-in-1 the folio keyboard shares the single USB HID
touchpad device (`044e:1218`, one interface, `bInterfaceClass=03`). It exposes
no evdev keyboard: `usb_conf` has one interface and `/proc/bus/input/devices`
shows only `Alps Alps Touchpad Mouse` / `Alps Alps Touchpad` / `Alps Alps
Touchpad UNKNOWN`, none with `kbd` handler or `KEY_A..KEY_Z`. Folding the folio
360 degrees does not cut pogo-pin power, so `044e:1218` stays enumerated and
indistinguishable from laptop posture whenever `SW_TABLET_MODE` is stale.

Consequence: while the folio is attached, auto-show stays suppressed in all
postures, including 360-degree tablet mode. Use `hyprosk toggle` or
`hyprosk hide` in that posture (both bypass folio gating by design).

### Sleep/wake check

After `s2idle` or hibernation, confirm the folio is still powered and not left
suspended by USB autosuspend (`usbcore.autosuspend=2` is set in
`/proc/cmdline`):

```sh
for f in control runtime_status runtime_enabled; do
  printf '%s: %s\n' "$f" "$(cat /sys/bus/usb/devices/1-4/power/$f)"
done
```

`control` must be `on` and `runtime_status` `active`. `hyprosk status` must
keep reporting `folio USB (044e:1218): true` after a resume; if it reports
`false` while the folio is docked, the device dropped off the bus and
`self.usb_folio_present()` is genuinely observing a re-enumeration window.

## Cursor-position auto-placement check

On `input-method-v2` auto-show (`Activate`), HyprOsk determines placement exclusivity from the triggering input position when available:

- **Touchscreen activation**: HyprOsk reads the touchscreen's raw Y axis and normalizes it to the screen height. The **upper half** uses a floating overlay (`exclusive_zone = 0`); the **lower half** docks (`exclusive_zone = effective_keyboard_height`). If the device does not expose a usable Y axis, HyprOsk falls back to the configured `exclusive_zone`.
- **Pointer-driven activation (mouse/touchpad)**: When activation occurs without recent touch, HyprOsk samples Hyprland's global cursor coordinates:
  - **Upper half (`rel_y < 0.5`)**: Displays as a **floating overlay** (`exclusive_zone = 0`).
  - **Lower half (`rel_y > 0.5`)**: Displays as **docked** (`exclusive_zone = effective_keyboard_height`).
  - **Unpositioned center sentinel**: When cursor coordinates match the monitor's unpositioned center sentinel (`width / 2, height / 2`), HyprOsk falls back safely to `config.general.exclusive_zone`.

### Manual Verification Procedure

1. **Top-half touchscreen activation**:
   - Tap an input field in the upper half of the touchscreen.
   - Verify HyprOsk appears as a floating overlay (`exclusive_zone=0`):
     ```
     Auto-placement: touchscreen rel_y=0.XX -> upper half -> floating overlay (exclusive_zone=0)
     ```
2. **Bottom-half touchscreen activation**:
   - Tap an input field in the lower half of the touchscreen.
   - Verify HyprOsk requests docked space reservation (`exclusive_zone = effective_keyboard_height`):
     ```
     Auto-placement: touchscreen rel_y=0.XX -> lower half -> docked (exclusive_zone=<height>)
     ```
3. **Top-half pointer click**:
   - Click an input field in the upper half of the monitor using mouse or touchpad.
   - Verify HyprOsk appears as a floating overlay (`exclusive_zone=0`):
     ```
     Auto-placement best-effort inference: cursor=(X, Y) on monitor '...' (WxH at X,Y), rel_y=0.XX -> upper half -> floating overlay (exclusive_zone=0)
     ```
4. **Bottom-half pointer click**:
   - Click an input field in the lower half of the monitor using mouse or touchpad.
   - Verify HyprOsk requests docked space reservation (`exclusive_zone = effective_keyboard_height`):
     ```
     Auto-placement best-effort inference: cursor=(X, Y) on monitor '...' (WxH at X,Y), rel_y=0.XX -> lower half -> docked (exclusive_zone=<height>)
     ```
5. **Unpositioned center sentinel fallback**:
   - If the pointer has never moved and sits at `width/2, height/2`, verify fallback:
     ```
     Auto-placement: cursor=(X, Y) is at monitor '...' unpositioned center sentinel; falling back to configured exclusive_zone=<height or 0>
     ```
6. **Manual invocations**:
   - Run `hyprosk show` or `hyprosk toggle` via CLI or shortcut.
   - Verify that manual show respects the user-configured `exclusive_zone` in `config.toml` directly, without cursor inference override.

If a daemon was killed abruptly, start the next one normally. HyprOsk detects
and reclaims the stale IPC socket.

## Comment style

Keep source comments short and concrete. Explain one reason or invariant; do
not narrate code that is already clear.
