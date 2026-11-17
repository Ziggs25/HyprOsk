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

## Responsive clipboard grid check

1. Open clipboard history and resize the keyboard through phone, tablet, and
   wide landscape widths. Cards should reflow from one to four columns, with
   four columns at 1920px, a 12px inset from each panel edge, and a 12px
   gap between cards. The first card should sit 12px below the panel top; at
   maximum scroll, the last card should sit 12px above the panel bottom. Wide
   landscape cards should be wider.
2. Drag the card grid past the top and bottom. Cards must stay clipped between
   the fixed header and footer; tapping the header or footer must never select a
   card underneath them.
3. Use a very short keyboard height. If the header and footer leave no card
   viewport, cards must be hidden and have no hit targets.

## Swipe typing check

FUTO-enabled builds use the `honorable_sturgeon` int8 encoder and a Rust CTC
beam decoder over references to the existing base and learned dictionary
entries. The model is loaded from `assets/futo_swipe/model_int8.pte`, the
installed `share/hyprosk/futo_swipe/model_int8.pte`, or `HYPROSK_FUTO_MODEL`.

Automated checks require the ExecuTorch 1.5.1 CMake output and `libclang` used
to build the FUTO feature:

```sh
nix-shell shell.nix --run \
  'EXECUTORCH_RS_EXECUTORCH_LIB_DIR=/path/to/executorch/cmake-out LIBCLANG_PATH=/path/to/libclang/lib cargo test --features futo-swipe -q swipe'
```

The swipe tests include FUTO's published `computer` trajectory and require the
model plus constrained decoder to rank `computer` first. Another test adds
`gyatt` to an isolated user dictionary and verifies that its prefixes and
terminal word are present in the swipe lexicon index.

Manual:

1. Run `hyprosk about` and verify the app displays “Swipe typing powered by
   FUTO Swipe technology.”
2. Hold a letter and glide across `hello`, `world`, `quick`, `brown`, and `fox`.
   While moving, show only the best live candidate in the center suggestion
   slot; ordinary autosuggest results should reappear after the gesture.
3. Teach HyprOsk a new word such as `gyatt` by selecting it from suggestions,
   then swipe that spelling. It should now be available to the decoder through
   the same user dictionary used by typed suggestions. Lift to commit the best
   candidate and trailing space; next-word predictions should follow where
   dictionary bigrams exist.
4. Verify the swipe trail is a solid, smooth marker stroke with a rounded fine
   tip at the oldest end, and fades completely about 420 ms after the last
   movement, including while a finger rests still. Its edges should look smooth
   over translucent and blurred layouts without dark seams or black fade tails.
   It uses colors.swipe_trail_color from the active layout JSON; if omitted, use
   the layout accent color. Set `swipe.trail.width` in the layout JSON and
   confirm the trail gets wider or narrower after switching layouts; omitted
   widths preserve the existing 7.2px default and configured widths clamp to
   1–48px. Check debug logs for preview decode timing; decodes
   over 100 ms trigger a warning. Preview updates are rate-limited to 10 per
   second.
5. Hold a letter without moving for more than 300 ms and verify its secondary-key
   popup and long-press output still work. After crossing the swipe threshold,
   the pressed-key highlight and popup should clear and no secondary character
   should be emitted. A short drag below half the starting key's width, or a drag
   starting on Space/Enter/Shift/digits, remains a normal tap.
6. Hold Shift or enable Caps Lock, swipe a word, and verify the committed word
   is capitalized or uppercased. One-shot Shift should be consumed as for a tap.
7. Put a second finger down during a drag and verify the swipe is cancelled and
   normal tap/chord handling resumes.
8. Resize the keyboard, switch to a custom layout, or rebuild a layer. Swipes
   must use the current key centers; the model lexicon should refresh only after
   the user dictionary changes, without copying word strings into C++.
9. Typing and tapping candidates normally must still follow the input-method-v2
   and autosuggest checks above.

To estimate the runtime cost, record `VmRSS` before and after the first swipe.
The first model execution and Rust reference-index creation happen on demand:

```sh
pid=$(pgrep -f 'target/(debug|release)/hyprosk' | head -1)
grep VmRSS /proc/$pid/status   # before the first swipe
# perform one letter swipe
grep VmRSS /proc/$pid/status  # after
```

The index contains word references, not a second string table. Measure this
separately from the model/runtime memory; do not compare the result with the
previous OpenSwipe C++ trie measurements.

## Folio-mode detection check

With `behavior.folio_mode = true`, run `hyprosk status` while the folio is
connected. Expect `folio USB (044e:1218): true`, `folio attached (verdict): true`,
`folio source: usb-044e:1218`, and `auto-show will be: SUPPRESSED`.

Source precedence is deliberate:

1. `044e:1218` (Alps Touchpad) present -> attached.
2. an **external** letter-capable keyboard present -> attached.
3. no external keyboard -> `SW_TABLET_MODE` active means detached.

The built-in `i8042` "AT Translated Set 2 keyboard"
(`/devices/platform/i8042/serio0/...`) and the `INT33D5:00` "Intel HID events" /
"Intel HID 5 button array" utilities carry `KEY_A..KEY_Z` **forever** — the
`i8042` device never unregisters when the folio is removed. They are reported as
`built-in i8042/acpi keyboard: true` and are classified out of step 2, so they
can never suppress auto-show on their own. Only an external keyboard (USB, or
the `serio1` docking-station PS/2 port) counts.

`SW_TABLET_MODE` is never allowed to *enable* suppression: on this Dell it comes
from `intel-hid` (INT33D5:00, "Intel HID switches") which only initializes the
switch on the first `0xcc`/`0xcd` ACPI notify, and the value can latch across
s2idle/hibernate. Because a flaky switch can never leave the user without any
keyboard, and never suppresses the OSK by itself, the failure mode is safe in
both directions.

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
