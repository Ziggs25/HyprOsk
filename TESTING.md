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

If a daemon was killed abruptly, start the next one normally. HyprOsk detects
and reclaims the stale IPC socket.

## Comment style

Keep source comments short and concrete. Explain one reason or invariant; do
not narrate code that is already clear.
