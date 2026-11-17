# Implementation plan

## Architecture

- Keep simulation and configuration independent of the Wayland adapter.
- Use SCTK layer-shell surfaces, one per output, with empty pointer regions.
- Use wgpu for GPU particle rendering; use Hyprland JSON IPC/events for window geometry.
- Keep snow depth as a per-surface height map; parse TOML from the XDG config directory.

## Milestones

1. **Foundation — implemented:** reproducible Rust/Nix project, SCTK layer-shell surfaces, wgpu rendering, and click-through input regions. Live-tested one Hyprland output at scale 1; multi-output paths are implemented but not yet tested on multiple displays.
2. **Snow — implemented:** deterministic particle motion, wind, monitor-edge height-map accumulation, compaction, and sliding are covered by unit tests. Visual scale/performance tuning remains.
3. **Hyprland integration — in progress:** Hyprland JSON IPC events and periodic geometry snapshots track visible windows. Independent in-memory workspace simulations restore accumulation on return, preserve snow overlapping windows, and release closed-window snow as flakes. Movement/resizing and switching are covered by IPC and simulation tests; live compositor scenarios remain to be verified.
4. **Physics and interaction — partial:** snow can be gathered into draggable, stackable balls using snow-shaped pointer input regions, without a separate interaction mode. Physics and region behavior have unit tests; real pointer interaction remains to be tested.
5. **Christmas controls — partial:** stacked balls automatically form draggable decorated snowmen; the snowman CLI command and configuration reload remain outstanding.
6. **Polish:** performance checks, multi-output/fullscreen testing, packaging, and documentation.

## Environment notes

- Development session: Hyprland 0.57.1, Wayland, one 1920x1080 eDP-1 output at scale 1 and 144 Hz.
- Workspaces 1, 2, and 3 are active; listed windows are tiled, and QuickShell occupies background and top layers.
- The host has Nix but no Rust toolchain on PATH; use the project dev shell to build.
- Wayland layer-shell gives placement and input policy but does not permit reading other clients' surfaces.

## Progress

- Live-tested the transparent GPU surface and snowfall on the active Hyprland output; HyprMCP reported the app in the `top` layer, and the active window remained unchanged.
- The IPC startup snapshot and event watcher run against the current Hyprland session; window shelves are covered by simulation unit tests. Dedicated compositor-driven window movement/resizing, fullscreen, and click-through interaction tests remain outstanding.
- The surface requests no keyboard focus. When interaction is enabled, only accumulated snow and snowballs are in its input region; live pointer click-through and dragging remain unverified.
- Decorated snowman assembly has simulation and rendering tests; live visual and pointer validation remains outstanding. Persistent accumulation, runtime config reload, and multi-monitor/mixed-scale runtime validation remain outstanding.
- Visited workspaces continue simulating on their associated monitor; first visits inherit a separately simulated background floor. Inactive window geometry is reconciled on return.
- Window snow approximates Hyprland's global corner rounding, and snowballs/snowmen collide with window tops and sides; fast side impacts release falling snow, while double-clicks make a non-accumulating burst. Simulation and geometry tests pass; live visual and pointer testing remain outstanding.
- Snowballs now use procedural GPU lighting and fine texture, with softer flakes and shaded accumulation. GPU initialization was smoke-tested; a visual comparison of snowmen and deep snow remains outstanding.
