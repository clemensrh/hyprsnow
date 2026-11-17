# Christmas Snow

A native Rust snowfall overlay for Hyprland on Wayland. It creates a transparent
top-layer surface per connected output, renders particles with wgpu, simulates
bottom-edge accumulation, and tracks visible window geometry through Hyprland IPC
to accumulate snow on window tops.
Snowballs use softly lit, procedural GPU shading, while accumulated snow has
bright edges and cool shaded depths; the effect does not use external images.

## Development

```sh
nix develop
cargo test
cargo build --release
```

The Nix development shell supplies Rust, pkg-config, Wayland, libxkbcommon, and
the Vulkan loader. The Nix package wraps the binary with the Vulkan and OpenGL
loader libraries needed at runtime; a compatible GPU driver must also be
installed and enabled. The runtime requires Hyprland with
`wlr-layer-shell-unstable-v1`.

## Install on NixOS

The flake exports a package and a NixOS module:

```nix
{
  inputs.christmas-snow.url = "path:/path/to/hyprsnow";
  imports = [ christmas-snow.nixosModules.default ];
  programs.christmas-snow.enable = true;
}
```

Or install `.#default` in `environment.systemPackages`, then install and enable
the included user units:

```sh
systemctl --user enable --now christmas-snow.service christmas-snow-tray.service
```

The NixOS module enables both the overlay and tray automatically. A Hyprland autostart example is in
[examples/hyprland.conf](./examples/hyprland.conf); the unit templates are
[systemd/christmas-snow.service](./systemd/christmas-snow.service) and
[systemd/christmas-snow-tray.service](./systemd/christmas-snow-tray.service).

## Commands

`start`, `stop`, `restart`, `status`, and `toggle` control
`christmas-snow.service`. The `tray` command runs the system tray icon and menu.
The tray provides overlay start/stop, snowfall and accumulation toggles, density,
wind and particle-size presets, and a shortcut to the configuration file.
Use right-button drag across accumulated snow to shovel it away, or choose
**Clear accumulated snow** to remove all floor and window-top accumulation from
every workspace. The tray clear leaves falling flakes and snowballs untouched.
Settings changes are saved to `config.toml` and restart the overlay when it is
running; restarting clears the in-memory snow and snowballs. The service runs
the internal `run` command.
The tray icon uses StatusNotifierItem and requires a compatible tray host in
the desktop panel.
`config` prints the merged current settings. The `snowman` command is not
implemented; snowmen form automatically through interaction instead.

To diagnose service startup failures, run
`journalctl --user -u christmas-snow.service -b --no-pager`.

## Configuration

Configuration is read from `$XDG_CONFIG_HOME/christmas-snow/config.toml`, or
`~/.config/christmas-snow/config.toml` when `XDG_CONFIG_HOME` is unset. Missing
configuration uses validated defaults documented in
[config/default.toml](./config/default.toml).

Window-top snow can be disabled with `windows.accumulate_on_windows = false`.
Set `windows.max_snow_depth` (pixels, `0`–`500`, default `32`) to control its
maximum depth independently of bottom-edge snow.

## Snowballs

Hold the left mouse button on accumulated snow and drag across it to gather
snow into a ball. Continue dragging over snow to make it larger; release to
drop it. Grab an existing ball to move or stack it on other balls. Balls also
fall onto tracked window tops and the bottom of the screen. A quick release
continues at the pointer's last measured velocity, then gravity, air resistance,
and surface friction slow and settle the ball. Holding still before release
drops it without a throw.
Hold the right mouse button on accumulated snow and drag to clear a path.
A shovel appears while held and faces the direction of horizontal movement.

Two gently stacked balls with radii of at least 10 simulation pixels form a
snowman; place a third above or below to add it. Balls should be approximately
centered; they are automatically reordered from largest at the bottom to
smallest at the top. The assembled balls move
and fall together when grabbed by any ball. Each snowman gets a face, carrot,
buttons, stick arms, hat, and scarf; its hat band and scarf are independently
chosen red or green. Decorations themselves are click-through.
Double-click a ball or any ball in a snowman to make it burst without adding snow.
A fast collision with a window side also breaks the ball or snowman; slower
impacts stop at the edge. The released snow settles on window tops or the
bottom, subject to the configured accumulation limits.

The overlay receives pointer events only over visible accumulated snow and
snowballs. Clicks in those areas do **not** reach the window or desktop
underneath; other areas remain click-through except while a shovel drag is
held, when the overlay captures input across that monitor until release.
The drag must start on accumulated snow or a snowball, including on an empty
workspace; the shovel cannot be activated by right-clicking a click-through area.
Disable this interaction in the configuration:

```toml
[interaction]
snowman_building = false
```

This also disables right-button shoveling.

Balls and snow are kept in memory per workspace and are lost on restart.
Cross-monitor dragging is not supported.

## Workspaces and windows

Each Hyprland workspace keeps its own in-memory snowfall and accumulation;
switching back restores its previous state and reconciles it with the windows
currently visible there. Previously visited workspaces continue simulating
while another workspace is displayed on the same monitor. A first visit starts
with that monitor's background snow layer, rather than an empty floor.
Inactive windows use their last known geometry until the workspace is visible
again; newly discovered windows start accumulating when they are first tracked.
Simulation pauses on a monitor when fullscreen pausing is enabled. Keeping
more workspaces active increases CPU usage. Special workspaces have their own
state while open.
Snow overlapping a newly visible or moved window is preserved and remains
visible on the overlay; snow from closed windows is released as falling flakes.
Snowballs released inside a window continue falling from their release position.
Window tops and sides remain collision surfaces when approached from outside.
Window-top snow follows the global Hyprland `decoration:rounding`
setting, approximating rounded corners; per-window rounding overrides and
non-circular corner shapes are not available through this geometry model.
Moving a window sideways sheds the snow left unsupported by its edge;
moving it downward releases its snow so it can fall back onto the window.
Dislodged accumulated snow falls faster than individual flakes.
It settles on window tops or the screen bottom, spreading across nearby
columns. If a window shelf reaches its configured depth limit, excess snow
continues downward; bottom accumulation is also subject to its depth limit.
Snow is not saved across application restarts.

Hyprland IPC events trigger geometry updates, supplemented by a 100 ms snapshot
interval because the event stream does not report every interactive move and
resize. The actual update latency also depends on IPC and frame scheduling.

## Current scope

The renderer supports transparent multi-output layer surfaces, snowfall, and
in-memory height maps for monitor bottoms and tracked window tops. Hyprland IPC
snapshots and events update eligible window shelves, while fullscreen monitor
state can pause rendering. The overlay uses a snow-shaped input region when
interaction is enabled; automated live pointer testing is still outstanding.
Accumulation is not persisted across restarts, and runtime configuration reload
and multi-monitor/mixed-scale testing remain unimplemented or unverified.
Snowman appearance and live pointer interactions have not yet been verified
on the compositor. Wayland does not allow reading other applications'
rendered contents; window tracking uses geometry-only Hyprland IPC.
