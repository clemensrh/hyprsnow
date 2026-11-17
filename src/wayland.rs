use crate::{
    config::Settings,
    control::{self, ControlRequest, ControlSocket},
    hyprland::{HyprTracker, MonitorSnapshot},
    input_region::{self, Rect},
    render::{GpuRenderer, Shovel, ShovelDirection},
    sim::WindowShelfInput,
    workspaces::Workspaces,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
};
use std::{
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_region, wl_seat, wl_surface},
};

const SHOVEL_RADIUS: f32 = 15.0;

struct Overlay {
    renderer: Option<GpuRenderer>,
    layer: LayerSurface,
    output: wl_output::WlOutput,
    monitor_name: Option<String>,
    workspace: Option<i64>,
    logical_width: u32,
    logical_height: u32,
    scale: i32,
    last_frame: Instant,
    fullscreen: bool,
    paused: bool,
    needs_redraw: bool,
    input_region: Vec<Rect>,
    input_region_scratch: Vec<Rect>,
}

impl Overlay {
    fn new(layer: LayerSurface, output: wl_output::WlOutput) -> Self {
        Self {
            renderer: None,
            layer,
            output,
            monitor_name: None,
            workspace: None,
            logical_width: 1,
            logical_height: 1,
            scale: 1,
            last_frame: Instant::now(),
            fullscreen: false,
            paused: false,
            needs_redraw: true,
            input_region: Vec::new(),
            input_region_scratch: Vec::new(),
        }
    }

    fn pixel_size(&self) -> (u32, u32) {
        (
            self.logical_width.saturating_mul(self.scale as u32),
            self.logical_height.saturating_mul(self.scale as u32),
        )
    }
}

struct App {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    overlays: Vec<Overlay>,
    settings: Settings,
    hypr_updates: Receiver<Result<Vec<MonitorSnapshot>, String>>,
    control_updates: Receiver<ControlRequest>,
    monitors: Vec<MonitorSnapshot>,
    shelves_dirty: bool,
    workspaces: Workspaces,
    pointers: Vec<(wl_seat::WlSeat, wl_pointer::WlPointer)>,
    drag: Option<PointerDrag>,
    last_click: Option<BallClick>,
    exit: bool,
    fatal_error: Option<String>,
}

struct PointerDrag {
    pointer: wl_pointer::WlPointer,
    surface: wl_surface::WlSurface,
    workspace: i64,
    last_time: u32,
    press_time: u32,
    press_position: (f32, f32),
    last_position: (f32, f32),
    kind: PointerDragKind,
    ball: Option<usize>,
    generation: u64,
    moved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerDragKind {
    Ball,
    Shovel(ShovelDirection),
}

struct BallClick {
    pointer: wl_pointer::WlPointer,
    surface: wl_surface::WlSurface,
    workspace: i64,
    time: u32,
    position: (f32, f32),
    ball: usize,
    generation: u64,
}

fn click_distance_and_time_match(
    previous_time: u32,
    previous: (f32, f32),
    time: u32,
    current: (f32, f32),
    scale: i32,
) -> bool {
    time.wrapping_sub(previous_time) <= 400
        && (current.0 - previous.0).hypot(current.1 - previous.1) <= 8.0 * scale as f32
}

impl App {
    fn fail(&mut self, error: String) {
        self.fatal_error = Some(error);
        self.exit = true;
    }

    fn cancel_drag(&mut self) {
        if let Some(drag) = self.drag.take()
            && let Some(overlay) = self
                .overlays
                .iter_mut()
                .find(|overlay| *overlay.layer.wl_surface() == drag.surface)
        {
            overlay.needs_redraw = true;
            if drag.kind == PointerDragKind::Ball {
                let (width, height) = overlay.pixel_size();
                self.workspaces
                    .get(drag.workspace, width, height)
                    .end_drag();
            }
        }
    }

    fn add_output(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if self.overlays.iter().any(|overlay| overlay.output == output) {
            return;
        }

        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Top,
            Some("christmas-snow"),
            Some(&output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        let input_region = self.compositor.wl_compositor().create_region(qh, ());
        layer.set_input_region(Some(&input_region));
        input_region.destroy();
        layer.commit();

        let mut overlay = Overlay::new(layer, output.clone());
        if let Some(info) = self.output_state.info(&output) {
            overlay.scale = info.scale_factor.max(1);
            overlay.monitor_name = info.name;
        }
        overlay.layer.wl_surface().set_buffer_scale(overlay.scale);
        self.overlays.push(overlay);
        self.shelves_dirty = true;
    }

    fn draw(&mut self, index: usize, connection: &Connection, qh: &QueueHandle<Self>) {
        self.process_control_requests();
        self.refresh_window_shelves();
        if self.exit {
            return;
        }
        if self.drag.as_ref().is_some_and(|drag| {
            drag.surface == *self.overlays[index].layer.wl_surface()
                && ((self.settings.general.pause_on_fullscreen && self.overlays[index].fullscreen)
                    || self.overlays[index].workspace.is_none())
        }) {
            self.cancel_drag();
        }
        let fps_limit = self.settings.general.fps_limit;
        let interval = Duration::from_secs_f64(1.0 / fps_limit as f64);
        let shovel = self.drag.as_ref().and_then(|drag| match drag.kind {
            PointerDragKind::Shovel(direction)
                if drag.surface == *self.overlays[index].layer.wl_surface()
                    && Some(drag.workspace) == self.overlays[index].workspace =>
            {
                Some(Shovel {
                    position: drag.last_position,
                    direction,
                    scale: self.overlays[index].scale as f32,
                })
            }
            _ => None,
        });
        let overlay = &mut self.overlays[index];
        let surface = overlay.layer.wl_surface();
        surface.frame(qh, FrameCallbackData(surface.clone()));
        let now = Instant::now();
        let pause = (self.settings.general.pause_on_fullscreen && overlay.fullscreen)
            || overlay.workspace.is_none();
        let redraw =
            !pause && (overlay.needs_redraw || now.duration_since(overlay.last_frame) >= interval);
        let (width, height) = overlay.pixel_size();
        if pause != overlay.paused || redraw {
            if !pause && self.settings.interaction.snowman_building {
                input_region::regions_into(
                    &mut overlay.input_region_scratch,
                    self.workspaces.get(
                        overlay.workspace.expect("active workspace"),
                        width,
                        height,
                    ),
                    overlay.scale,
                    overlay.logical_width,
                    overlay.logical_height,
                    shovel.is_some(),
                );
            } else {
                overlay.input_region_scratch.clear();
            }
            if overlay.input_region_scratch != overlay.input_region {
                let region = self.compositor.wl_compositor().create_region(qh, ());
                for rect in &overlay.input_region_scratch {
                    region.add(rect.x, rect.y, rect.width, rect.height);
                }
                overlay.layer.set_input_region(Some(&region));
                region.destroy();
                std::mem::swap(&mut overlay.input_region, &mut overlay.input_region_scratch);
            }
        }
        if pause && !overlay.paused {
            if let Some(renderer) = &mut overlay.renderer
                && let Err(error) = renderer.clear()
            {
                self.fail(error);
                return;
            }
            overlay.paused = true;
        } else if redraw {
            let dt = now.duration_since(overlay.last_frame).as_secs_f32();
            overlay.last_frame = now;
            overlay.paused = false;
            overlay.needs_redraw = false;
            self.workspaces
                .step_monitor(overlay.monitor_name.as_deref().expect("active monitor"), dt);
            let simulation =
                self.workspaces
                    .get(overlay.workspace.expect("active workspace"), width, height);
            if let Some(renderer) = &mut overlay.renderer
                && let Err(error) = renderer.render(simulation, shovel)
            {
                self.fail(error);
                return;
            }
        }
        overlay.layer.commit();
        if let Err(error) = connection.flush() {
            self.fail(format!("cannot flush Wayland requests: {error}"));
        }
    }

    fn process_control_requests(&mut self) {
        loop {
            match self.control_updates.try_recv() {
                Ok(ControlRequest::ClearAccumulatedSnow) => {
                    self.workspaces.clear_accumulated_snow();
                    for overlay in &mut self.overlays {
                        overlay.needs_redraw = true;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.fail("control socket listener stopped".to_owned());
                    break;
                }
            }
        }
    }

    fn refresh_window_shelves(&mut self) {
        let mut changed = std::mem::take(&mut self.shelves_dirty);
        loop {
            match self.hypr_updates.try_recv() {
                Ok(Ok(monitors)) if self.monitors != monitors => {
                    self.monitors = monitors;
                    changed = true;
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("christmas-snow: {error}"),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.fail("Hyprland IPC watcher stopped".to_owned());
                    return;
                }
            }
        }
        if !changed {
            return;
        }

        for overlay in &mut self.overlays {
            let Some(name) = overlay.monitor_name.as_deref() else {
                overlay.workspace = None;
                continue;
            };
            let Some(monitor) = self.monitors.iter().find(|monitor| monitor.name == name) else {
                overlay.workspace = None;
                continue;
            };
            overlay.fullscreen = monitor.fullscreen;
            let switched = overlay.workspace != Some(monitor.workspace);
            if switched {
                if self
                    .drag
                    .as_ref()
                    .is_some_and(|drag| drag.surface == *overlay.layer.wl_surface())
                    && let Some(drag) = self.drag.take()
                {
                    let (width, height) = overlay.pixel_size();
                    self.workspaces
                        .get(drag.workspace, width, height)
                        .end_drag();
                }
                overlay.workspace = Some(monitor.workspace);
                overlay.last_frame = Instant::now();
                overlay.needs_redraw = true;
            }
            let windows: Vec<_> = if self.settings.windows.enabled {
                monitor
                    .windows
                    .iter()
                    .map(|window| WindowShelfInput {
                        id: window.address.clone(),
                        x: (window.x - monitor.x) * overlay.scale as f32,
                        y: (window.y - monitor.y) * overlay.scale as f32,
                        width: window.width * overlay.scale as f32,
                        height: window.height * overlay.scale as f32,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let (width, height) = overlay.pixel_size();
            self.workspaces
                .activate(monitor.workspace, name, width, height);
            let simulation = self.workspaces.get(monitor.workspace, width, height);
            simulation.set_window_rounding(monitor.rounding * overlay.scale as f32);
            simulation.set_windows(&windows, self.settings.windows.accumulate_on_windows);
        }
    }

    fn configure(
        &mut self,
        index: usize,
        connection: &Connection,
        configure: LayerSurfaceConfigure,
    ) {
        let overlay = &mut self.overlays[index];
        let output_size = self
            .output_state
            .info(&overlay.output)
            .and_then(|info| info.logical_size);
        overlay.logical_width = if configure.new_size.0 > 0 {
            configure.new_size.0
        } else {
            output_size.map_or(1, |(width, _)| width.max(1) as u32)
        };
        overlay.logical_height = if configure.new_size.1 > 0 {
            configure.new_size.1
        } else {
            output_size.map_or(1, |(_, height)| height.max(1) as u32)
        };
        if let Some(info) = self.output_state.info(&overlay.output) {
            overlay.scale = info.scale_factor.max(1);
            overlay.monitor_name = info.name;
        }
        self.shelves_dirty = true;
        let (width, height) = overlay.pixel_size();
        eprintln!(
            "christmas-snow: output configured at {}x{} logical, scale {}",
            overlay.logical_width, overlay.logical_height, overlay.scale
        );
        if let Some(renderer) = &mut overlay.renderer {
            renderer.resize(width, height);
        } else {
            match GpuRenderer::new(connection, overlay.layer.wl_surface(), width, height) {
                Ok(renderer) => {
                    eprintln!(
                        "christmas-snow: GPU renderer ready ({})",
                        renderer.adapter_name()
                    );
                    overlay.renderer = Some(renderer);
                }
                Err(error) => {
                    self.fail(error);
                }
            }
        }
    }
}

pub fn run(settings: Settings) -> Result<(), String> {
    let (_control_socket, control_updates): (ControlSocket, _) = control::start_listener()?;
    let tracker = HyprTracker::start(settings.windows.clone())?;
    let connection = Connection::connect_to_env()
        .map_err(|error| format!("cannot connect to the Wayland compositor: {error}"))?;
    let (globals, mut event_queue) = registry_queue_init(&connection)
        .map_err(|error| format!("cannot read Wayland globals: {error}"))?;
    let qh = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|_| "the compositor does not provide wl_compositor")?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|_| "the compositor does not provide wlr-layer-shell")?;

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        overlays: Vec::new(),
        workspaces: Workspaces::new(&settings),
        settings,
        hypr_updates: tracker.updates,
        control_updates,
        monitors: Vec::new(),
        shelves_dirty: true,
        pointers: Vec::new(),
        drag: None,
        last_click: None,
        exit: false,
        fatal_error: None,
    };

    event_queue
        .roundtrip(&mut app)
        .map_err(|error| format!("Wayland initial roundtrip failed: {error}"))?;
    let outputs: Vec<_> = app.output_state.outputs().collect();
    for output in outputs {
        app.add_output(&qh, output);
    }
    if app.overlays.is_empty() {
        return Err("the compositor reported no connected outputs".to_owned());
    }
    connection
        .flush()
        .map_err(|error| format!("cannot flush Wayland requests: {error}"))?;
    eprintln!(
        "christmas-snow: starting {} transparent monitor overlay(s)",
        app.overlays.len()
    );

    while !app.exit {
        event_queue
            .blocking_dispatch(&mut app)
            .map_err(|error| format!("Wayland event dispatch failed: {error}"))?;
    }
    match app.fatal_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if let Some(overlay) = self
            .overlays
            .iter_mut()
            .find(|overlay| overlay.layer.wl_surface() == surface)
        {
            overlay.scale = new_factor.max(1);
            overlay.layer.wl_surface().set_buffer_scale(overlay.scale);
            let (width, height) = overlay.pixel_size();
            if let Some(renderer) = &mut overlay.renderer {
                renderer.resize(width, height);
            }
            self.shelves_dirty = true;
        }
    }

    fn transform_changed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        connection: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        if let Some(index) = self
            .overlays
            .iter()
            .position(|overlay| overlay.layer.wl_surface() == surface)
        {
            self.draw(index, connection, qh);
        }
    }

    fn surface_enter(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _connection: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.add_output(qh, output);
    }

    fn update_output(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output)
            && let Some(overlay) = self
                .overlays
                .iter_mut()
                .find(|overlay| overlay.output == output)
            && overlay.scale != info.scale_factor.max(1)
        {
            overlay.scale = info.scale_factor.max(1);
            self.shelves_dirty = true;
        }
    }

    fn output_destroyed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.overlays.iter().any(|overlay| {
            overlay.output == output
                && self
                    .drag
                    .as_ref()
                    .is_some_and(|drag| drag.surface == *overlay.layer.wl_surface())
        }) {
            self.cancel_drag();
        }
        self.overlays.retain(|overlay| overlay.output != output);
        if self.overlays.is_empty() {
            self.exit = true;
        }
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _connection: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if self
            .drag
            .as_ref()
            .is_some_and(|drag| drag.surface == *layer.wl_surface())
        {
            self.cancel_drag();
        }
        self.overlays
            .retain(|overlay| overlay.layer.wl_surface() != layer.wl_surface());
        if self.overlays.is_empty() {
            self.exit = true;
        }
    }

    fn configure(
        &mut self,
        connection: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if let Some(index) = self
            .overlays
            .iter()
            .position(|overlay| overlay.layer.wl_surface() == layer.wl_surface())
        {
            self.configure(index, connection, configure);
            if !self.exit {
                self.draw(index, connection, qh);
            }
        }
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
    ) {
    }

    fn new_capability(
        &mut self,
        _connection: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && !self.pointers.iter().any(|(s, _)| *s == seat) {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => self.pointers.push((seat, pointer)),
                Err(error) => self.fail(format!("cannot initialize pointer: {error}")),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && let Some(index) = self.pointers.iter().position(|(s, _)| *s == seat)
        {
            let (_, pointer) = self.pointers.remove(index);
            if self
                .drag
                .as_ref()
                .is_some_and(|drag| drag.pointer == pointer)
            {
                self.cancel_drag();
            }
            pointer.release();
        }
    }

    fn remove_seat(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
    ) {
        if let Some(index) = self.pointers.iter().position(|(s, _)| *s == seat) {
            let (_, pointer) = self.pointers.remove(index);
            if self
                .drag
                .as_ref()
                .is_some_and(|drag| drag.pointer == pointer)
            {
                self.cancel_drag();
            }
            pointer.release();
        }
    }
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        connection: &Connection,
        qh: &QueueHandle<Self>,
        pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        const LEFT_BUTTON: u32 = 0x110;
        const RIGHT_BUTTON: u32 = 0x111;
        for event in events {
            let Some(overlay) = self
                .overlays
                .iter_mut()
                .find(|overlay| *overlay.layer.wl_surface() == event.surface)
            else {
                continue;
            };
            let Some(workspace) = overlay.workspace else {
                continue;
            };
            let (width, height) = overlay.pixel_size();
            let x = event.position.0 as f32 * overlay.scale as f32;
            let y = event.position.1 as f32 * overlay.scale as f32;
            match event.kind {
                PointerEventKind::Press {
                    button: RIGHT_BUTTON,
                    time,
                    ..
                } if self.drag.is_none()
                    && self.settings.interaction.snowman_building
                    && !overlay.paused =>
                {
                    let rect =
                        input_region::full_surface(overlay.logical_width, overlay.logical_height);
                    let region = self.compositor.wl_compositor().create_region(qh, ());
                    region.add(rect.x, rect.y, rect.width, rect.height);
                    overlay.layer.set_input_region(Some(&region));
                    region.destroy();
                    overlay.layer.commit();
                    if let Err(error) = connection.flush() {
                        self.fail(format!("cannot flush shovel input region: {error}"));
                        return;
                    }
                    overlay.input_region.clear();
                    overlay.input_region.push(rect);
                    self.last_click = None;
                    self.workspaces.get(workspace, width, height).shovel_snow(
                        (x, y),
                        (x, y),
                        SHOVEL_RADIUS * overlay.scale as f32,
                    );
                    self.drag = Some(PointerDrag {
                        pointer: pointer.clone(),
                        surface: event.surface.clone(),
                        workspace,
                        last_time: time,
                        press_time: time,
                        press_position: (x, y),
                        last_position: (x, y),
                        kind: PointerDragKind::Shovel(ShovelDirection::Right),
                        ball: None,
                        generation: 0,
                        moved: true,
                    });
                    overlay.needs_redraw = true;
                }
                PointerEventKind::Press {
                    button: LEFT_BUTTON,
                    time,
                    ..
                } if self.drag.is_none()
                    && self.settings.interaction.snowman_building
                    && !overlay.paused =>
                {
                    let simulation = self.workspaces.get(workspace, width, height);
                    let ball = simulation.ball_at(x, y);
                    let generation = simulation.ball_generation();
                    let double = self.last_click.take().is_some_and(|click| {
                        click.pointer == *pointer
                            && click.surface == event.surface
                            && click.workspace == workspace
                            && Some(click.ball) == ball
                            && click.generation == generation
                            && click_distance_and_time_match(
                                click.time,
                                click.position,
                                time,
                                (x, y),
                                overlay.scale,
                            )
                    });
                    if double {
                        simulation
                            .explode_ball_without_snow(ball.expect("double click has a ball"));
                        continue;
                    }
                    if self
                        .workspaces
                        .get(workspace, width, height)
                        .begin_drag(x, y)
                    {
                        self.drag = Some(PointerDrag {
                            pointer: pointer.clone(),
                            surface: event.surface.clone(),
                            workspace,
                            last_time: time,
                            press_time: time,
                            press_position: (x, y),
                            last_position: (x, y),
                            kind: PointerDragKind::Ball,
                            ball,
                            generation,
                            moved: false,
                        });
                    }
                }
                PointerEventKind::Motion { time } => {
                    if let Some(drag) = &mut self.drag
                        && drag.pointer == *pointer
                        && drag.surface == event.surface
                        && drag.workspace == workspace
                    {
                        let dt = time.wrapping_sub(drag.last_time) as f32 / 1000.0;
                        drag.last_time = time;
                        let previous_position = drag.last_position;
                        if let PointerDragKind::Shovel(direction) = drag.kind {
                            drag.kind = PointerDragKind::Shovel(
                                direction.after_motion(previous_position.0, x),
                            );
                        }
                        let kind = drag.kind;
                        drag.last_position = (x, y);
                        if (x - drag.press_position.0).hypot(y - drag.press_position.1)
                            > 8.0 * overlay.scale as f32
                        {
                            drag.moved = true;
                        }
                        match kind {
                            PointerDragKind::Ball => self
                                .workspaces
                                .get(workspace, width, height)
                                .move_drag(x, y, dt),
                            PointerDragKind::Shovel(_) => {
                                self.workspaces.get(workspace, width, height).shovel_snow(
                                    previous_position,
                                    (x, y),
                                    SHOVEL_RADIUS * overlay.scale as f32,
                                );
                                overlay.needs_redraw = true;
                            }
                        }
                    }
                }
                PointerEventKind::Release {
                    button: LEFT_BUTTON,
                    time,
                    ..
                } => {
                    if let Some(drag) = &self.drag
                        && drag.pointer == *pointer
                        && drag.kind == PointerDragKind::Ball
                    {
                        self.last_click =
                            if !drag.moved && time.wrapping_sub(drag.press_time) <= 400 {
                                drag.ball.map(|ball| BallClick {
                                    pointer: drag.pointer.clone(),
                                    surface: drag.surface.clone(),
                                    workspace: drag.workspace,
                                    time,
                                    position: (x, y),
                                    ball,
                                    generation: drag.generation,
                                })
                            } else {
                                None
                            };
                        if time.wrapping_sub(drag.last_time) > 120 {
                            self.workspaces
                                .get(workspace, width, height)
                                .stop_drag_motion();
                        }
                        self.cancel_drag();
                    }
                }
                PointerEventKind::Release {
                    button: RIGHT_BUTTON,
                    ..
                } if self.drag.as_ref().is_some_and(|drag| {
                    drag.pointer == *pointer && matches!(drag.kind, PointerDragKind::Shovel(_))
                }) =>
                {
                    self.cancel_drag();
                }
                PointerEventKind::Leave { .. }
                    if self
                        .drag
                        .as_ref()
                        .is_some_and(|drag| drag.pointer == *pointer) =>
                {
                    self.last_click = None;
                    self.cancel_drag();
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_region::WlRegion, ()> for App {
    fn event(
        _state: &mut Self,
        _proxy: &wl_region::WlRegion,
        _event: wl_region::Event,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::click_distance_and_time_match;
    #[test]
    fn double_click_timing_distance_and_scale_are_bounded() {
        assert!(click_distance_and_time_match(
            100,
            (10.0, 10.0),
            500,
            (18.0, 10.0),
            1
        ));
        assert!(!click_distance_and_time_match(
            100,
            (10.0, 10.0),
            501,
            (10.0, 10.0),
            1
        ));
        assert!(!click_distance_and_time_match(
            100,
            (10.0, 10.0),
            200,
            (19.0, 10.0),
            1
        ));
        assert!(click_distance_and_time_match(
            100,
            (10.0, 10.0),
            200,
            (26.0, 10.0),
            2
        ));
        assert!(click_distance_and_time_match(
            u32::MAX - 10,
            (0.0, 0.0),
            10,
            (0.0, 0.0),
            1
        ));
    }
}

delegate_registry!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(App);
