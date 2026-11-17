const RELEASED_SNOW_INITIAL_SPEED: f32 = 110.0;
const RELEASED_SNOW_TERMINAL_SPEED: f32 = 300.0;
const RELEASED_SNOW_GRAVITY_MULTIPLIER: f32 = 28.0;
const SNOWMAN_MIN_RADIUS: f32 = 10.0;
const SNOWBALL_GROUND_FRICTION: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flake {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub radius: f32,
    pub opacity: f32,
    pub phase: f32,
    pub shape: u8,
    pub snow_mass: Option<f32>,
}

fn shovel_surface(
    depths: &mut [f32],
    origin: f32,
    center_x: f32,
    y: f32,
    radius: f32,
    baseline: impl Fn(f32) -> f32,
) {
    let start = ((center_x - radius - origin).floor() as isize).max(0) as usize;
    let end = ((center_x + radius - origin).ceil() as usize).min(depths.len());
    for (index, depth) in depths.iter_mut().enumerate().take(end).skip(start) {
        if *depth <= 0.0 {
            continue;
        }
        let x = origin + index as f32 + 0.5;
        let bottom = baseline(x);
        let top = bottom - *depth;
        let nearest_y = y.clamp(top, bottom);
        let horizontal_distance = x - center_x;
        if horizontal_distance * horizontal_distance + (y - nearest_y).powi(2) <= radius * radius {
            *depth = 0.0;
        }
    }
}

#[derive(Clone, Debug)]
pub struct WindowShelf {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub rounding: f32,
    pub snow_depth: Vec<f32>,
}

impl WindowShelf {
    pub fn baseline(&self, x: f32) -> f32 {
        rounded_baseline(self.x, self.y, self.snow_depth.len(), self.rounding, x)
    }
}

fn rounded_baseline(x: f32, y: f32, width: usize, rounding: f32, at: f32) -> f32 {
    let radius = rounding.min(width as f32 * 0.5).max(0.0);
    if radius == 0.0 {
        return y;
    }
    let local = (at - x).clamp(0.0, width as f32);
    let distance = local.min(width as f32 - local);
    if distance >= radius {
        return y;
    }
    let dx = radius - distance;
    y + radius - (radius * radius - dx * dx).max(0.0).sqrt()
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowShelfInput {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug)]
pub struct Snowball {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub mass: f32,
}

impl Snowball {
    pub fn radius(&self) -> f32 {
        (self.mass / std::f32::consts::PI).sqrt()
    }
}

#[derive(Clone, Debug)]
pub struct Snowman {
    pub balls: Vec<usize>,
    pub red_hat_band: bool,
    pub red_scarf: bool,
}

#[derive(Clone, Debug)]
struct Drag {
    index: usize,
    offset_x: f32,
    offset_y: f32,
    pointer_x: f32,
    pointer_y: f32,
}

#[derive(Clone, Debug)]
pub struct Simulation {
    pub flakes: Vec<Flake>,
    flake_scratch: Vec<Flake>,
    deposit_scratch: Vec<(f32, Option<f32>, f32)>,
    snowball_position_scratch: Vec<(f32, f32)>,
    snowball_group_scratch: Vec<Vec<usize>>,
    shattered_snowball_scratch: Vec<usize>,
    used_snowball_scratch: Vec<bool>,
    pub snow_depth: Vec<f32>,
    surface_transfers: Vec<f32>,
    pub window_shelves: Vec<WindowShelf>,
    pub snowballs: Vec<Snowball>,
    pub snowmen: Vec<Snowman>,
    drag: Option<Drag>,
    windows: Vec<WindowShelfInput>,
    window_rounding: f32,
    width: u32,
    height: u32,
    density: f32,
    gravity: f32,
    particle_size: f32,
    wind_strength: f32,
    accumulation_speed: f32,
    max_snow_depth: f32,
    max_window_snow_depth: f32,
    compaction: f32,
    sliding: bool,
    wind: f32,
    target_wind: f32,
    spawn_credit: f32,
    random: Random,
    ball_generation: u64,
    floor_capacity_warned: bool,
}

impl Simulation {
    pub fn new(
        width: u32,
        height: u32,
        density: f32,
        wind_strength: f32,
        max_snow_depth: f32,
    ) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Self {
            flakes: Vec::new(),
            flake_scratch: Vec::new(),
            deposit_scratch: Vec::new(),
            snowball_position_scratch: Vec::new(),
            snowball_group_scratch: Vec::new(),
            shattered_snowball_scratch: Vec::new(),
            used_snowball_scratch: Vec::new(),
            snow_depth: vec![0.0; width as usize],
            surface_transfers: Vec::new(),
            window_shelves: Vec::new(),
            snowballs: Vec::new(),
            snowmen: Vec::new(),
            drag: None,
            windows: Vec::new(),
            window_rounding: 0.0,
            width,
            height,
            density: density.clamp(0.0, 1.0),
            gravity: 9.81,
            particle_size: 1.0,
            wind_strength: wind_strength.clamp(0.0, 1.0),
            accumulation_speed: 1.0,
            max_snow_depth: max_snow_depth.max(0.0),
            max_window_snow_depth: max_snow_depth.max(0.0),
            compaction: 0.4,
            sliding: true,
            wind: 0.0,
            target_wind: 0.0,
            spawn_credit: 0.0,
            random: Random::new(0x9e37_79b9_7f4a_7c15),
            ball_generation: 0,
            floor_capacity_warned: false,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return;
        }
        self.snow_depth.resize(width as usize, 0.0);
        self.width = width;
        self.height = height;
        self.flakes
            .retain(|flake| flake.x >= 0.0 && flake.x < width as f32);
    }

    pub fn set_accumulation_speed(&mut self, speed: f32) {
        self.accumulation_speed = speed.clamp(0.0, 4.0);
    }

    pub fn set_window_snow_depth_limit(&mut self, depth: f32) {
        self.max_window_snow_depth = depth.clamp(0.0, 500.0);
        for shelf in &mut self.window_shelves {
            for value in &mut shelf.snow_depth {
                *value = (*value).min(self.max_window_snow_depth);
            }
        }
    }

    pub fn set_window_rounding(&mut self, rounding: f32) {
        self.window_rounding = rounding.max(0.0);
        for shelf in &mut self.window_shelves {
            let height = self
                .windows
                .iter()
                .find(|window| window.id == shelf.id)
                .map_or(f32::INFINITY, |window| window.height * 0.5);
            shelf.rounding = self.window_rounding.min(height);
        }
    }

    pub fn set_particle_size(&mut self, scale: f32) {
        self.particle_size = scale.clamp(0.25, 4.0);
    }

    pub fn set_gravity(&mut self, gravity: f32) {
        self.gravity = gravity.clamp(0.0, 50.0);
    }

    pub fn set_compaction(&mut self, compaction: f32) {
        self.compaction = compaction.clamp(0.0, 1.0);
    }

    pub fn set_sliding(&mut self, sliding: bool) {
        self.sliding = sliding;
    }

    pub fn particle_size(&self) -> f32 {
        self.particle_size
    }

    pub fn begin_drag(&mut self, x: f32, y: f32) -> bool {
        if self.drag.is_some() {
            return false;
        }
        let selected = self.ball_at(x, y);
        let index = if let Some(index) = selected {
            index
        } else {
            let amount = self.gather(x, y, 9.0, 3.0);
            if amount <= 0.0 {
                return false;
            }

            self.snowballs.push(Snowball {
                x,
                y,
                vx: 0.0,
                vy: 0.0,
                mass: amount,
            });
            self.snowballs.len() - 1
        };
        let members = self.snowman_members(index);
        for &member in &members {
            self.snowballs[member].vx = 0.0;
            self.snowballs[member].vy = 0.0;
        }
        let ball = &self.snowballs[index];
        self.drag = Some(Drag {
            index,
            offset_x: ball.x - x,
            offset_y: ball.y - y,
            pointer_x: x,
            pointer_y: y,
        });
        true
    }

    pub fn ball_at(&self, x: f32, y: f32) -> Option<usize> {
        self.snowballs
            .iter()
            .rposition(|ball| (ball.x - x).powi(2) + (ball.y - y).powi(2) <= ball.radius().powi(2))
    }

    pub fn ball_generation(&self) -> u64 {
        self.ball_generation
    }

    pub fn clear_accumulated_snow(&mut self) {
        self.snow_depth.fill(0.0);
        for shelf in &mut self.window_shelves {
            shelf.snow_depth.fill(0.0);
        }
    }

    pub fn shovel_snow(&mut self, from: (f32, f32), to: (f32, f32), radius: f32) {
        if !radius.is_finite() || radius <= 0.0 {
            return;
        }
        let distance = (to.0 - from.0).hypot(to.1 - from.1);
        let steps = (distance / (radius * 0.5)).ceil().max(1.0) as usize;
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let x = from.0 + (to.0 - from.0) * t;
            let y = from.1 + (to.1 - from.1) * t;
            let height = self.height as f32;
            shovel_surface(&mut self.snow_depth, 0.0, x, y, radius, |_| height);
            for shelf in &mut self.window_shelves {
                let shelf_x = shelf.x;
                let shelf_y = shelf.y;
                let shelf_width = shelf.snow_depth.len();
                let rounding = shelf.rounding;
                shovel_surface(&mut shelf.snow_depth, shelf_x, x, y, radius, |x| {
                    rounded_baseline(shelf_x, shelf_y, shelf_width, rounding, x)
                });
            }
        }
    }

    pub fn explode_ball_without_snow(&mut self, index: usize) {
        if index >= self.snowballs.len() {
            return;
        }
        let members = self.snowman_members(index);
        self.explode_members(&members, false);
    }

    fn explode_members(&mut self, members: &[usize], release_snow: bool) {
        let mut removed = members.to_vec();
        removed.sort_unstable();
        removed.dedup();
        if self
            .drag
            .as_ref()
            .is_some_and(|drag| removed.contains(&drag.index))
        {
            self.drag = None;
        }
        self.snowmen
            .retain(|man| !man.balls.iter().any(|index| removed.contains(index)));
        for &index in removed.iter().rev() {
            let ball = self.snowballs.remove(index);
            let count = (ball.radius() * 1.5).clamp(12.0, 48.0) as usize;
            for _ in 0..count {
                let angle = self.random.unit() * std::f32::consts::TAU;
                let speed = 35.0 + self.random.unit() * 100.0;
                self.flakes.push(Flake {
                    x: ball.x + angle.cos() * ball.radius() * 0.4,
                    y: ball.y + angle.sin() * ball.radius() * 0.4,
                    vx: ball.vx * 0.15 + angle.cos() * speed,
                    vy: ball.vy * 0.15 + angle.sin() * speed - 35.0,
                    radius: (ball.radius() * 0.12).clamp(1.0, 3.0),
                    opacity: 0.85,
                    phase: angle,
                    shape: 0,
                    snow_mass: Some(if release_snow {
                        ball.mass / count as f32
                    } else {
                        0.0
                    }),
                });
            }
        }
        for man in &mut self.snowmen {
            for index in &mut man.balls {
                *index -= removed.iter().filter(|&&removed| removed < *index).count();
            }
        }
        if let Some(drag) = &mut self.drag {
            drag.index -= removed.iter().filter(|&&index| index < drag.index).count();
        }
        self.ball_generation = self.ball_generation.wrapping_add(1);
    }

    pub fn move_drag(&mut self, x: f32, y: f32, dt: f32) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        let members = self.snowman_members(drag.index);
        let distance = (x - drag.pointer_x).hypot(y - drag.pointer_y);
        let steps = (distance / 4.0).ceil().clamp(1.0, 256.0) as usize;
        let mut gathered = 0.0;
        for i in 1..=if members.len() == 1 { steps } else { 0 } {
            let t = i as f32 / steps as f32;
            gathered += self.gather(
                drag.pointer_x + (x - drag.pointer_x) * t,
                drag.pointer_y + (y - drag.pointer_y) * t,
                9.0,
                0.5,
            );
        }
        let dx = x + drag.offset_x - self.snowballs[drag.index].x;
        let dy = y + drag.offset_y - self.snowballs[drag.index].y;
        self.snowballs[drag.index].mass += gathered;
        for index in members {
            let ball = &mut self.snowballs[index];
            if dt > 0.0 && dt < 0.25 && dt.is_finite() {
                ball.vx = (x - drag.pointer_x) / dt;
                ball.vy = (y - drag.pointer_y) / dt;
            } else {
                ball.vx = 0.0;
                ball.vy = 0.0;
            }
            ball.x += dx;
            ball.y += dy;
        }
        if let Some(drag) = &mut self.drag {
            drag.pointer_x = x;
            drag.pointer_y = y;
        }
    }

    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    pub fn stop_drag_motion(&mut self) {
        if let Some(drag) = &self.drag {
            for index in self.snowman_members(drag.index) {
                self.snowballs[index].vx = 0.0;
                self.snowballs[index].vy = 0.0;
            }
        }
    }

    fn snowman_members(&self, index: usize) -> Vec<usize> {
        self.snowmen
            .iter()
            .find(|man| man.balls.contains(&index))
            .map_or_else(|| vec![index], |man| man.balls.clone())
    }

    fn gather(&mut self, x: f32, y: f32, radius: f32, rate: f32) -> f32 {
        let mut amount = 0.0;
        fn take(
            depths: &mut [f32],
            origin: f32,
            baseline: impl Fn(f32) -> f32,
            x: f32,
            y: f32,
            radius: f32,
            rate: f32,
        ) -> f32 {
            let mut amount = 0.0;
            let start = (x - radius - origin).floor().max(0.0) as usize;
            let end = ((x + radius - origin).ceil().max(0.0) as usize).min(depths.len());
            for (index, depth) in depths.iter_mut().enumerate().take(end).skip(start) {
                let dx = origin + index as f32 + 0.5 - x;
                let baseline = baseline(origin + index as f32 + 0.5);
                let vertical = (radius * radius - dx * dx).max(0.0).sqrt();
                if y + vertical >= baseline - *depth && y - vertical <= baseline && *depth > 0.0 {
                    let removed = (*depth).min(rate);
                    *depth -= removed;
                    amount += removed;
                }
            }
            amount
        }
        amount += take(
            &mut self.snow_depth,
            0.0,
            |_| self.height as f32,
            x,
            y,
            radius,
            rate,
        );
        for shelf in &mut self.window_shelves {
            let origin = shelf.x;
            let baseline = shelf.y;
            let rounding = shelf.rounding;
            let width = shelf.snow_depth.len();
            amount += take(
                &mut shelf.snow_depth,
                origin,
                |x| rounded_baseline(origin, baseline, width, rounding, x),
                x,
                y,
                radius,
                rate,
            );
        }
        amount
    }

    pub fn set_windows(&mut self, inputs: &[WindowShelfInput], accumulate: bool) {
        if self.windows == inputs && (accumulate || self.window_shelves.is_empty()) {
            return;
        }
        let mut next = Vec::with_capacity(inputs.len());
        let mut released = Vec::new();
        for input in inputs.iter().filter(|_| accumulate) {
            let width = input.width.ceil().max(1.0) as usize;
            let old = self
                .window_shelves
                .iter()
                .find(|shelf| shelf.id == input.id);
            let mut depth = vec![0.0; width];
            if let Some(old) = old {
                if input.y > old.y || input.x != old.x {
                    for (index, &amount) in old.snow_depth.iter().enumerate() {
                        let x = old.x + index as f32 + 0.5;
                        let local = (x - input.x).floor();
                        if input.y <= old.y && local >= 0.0 && local < width as f32 {
                            depth[local as usize] += amount;
                        } else if amount > 0.0 {
                            released.push((x, old.y, amount));
                        }
                    }
                } else {
                    depth = resample(&old.snow_depth, width);
                }
            }
            for value in &mut depth {
                *value = (*value).clamp(0.0, self.max_window_snow_depth);
            }
            next.push(WindowShelf {
                id: input.id.clone(),
                x: input.x,
                y: input.y,
                rounding: self.window_rounding.min(input.height * 0.5),
                snow_depth: depth,
            });
        }
        next.sort_by(|left, right| left.x.total_cmp(&right.x));

        for shelf in self
            .window_shelves
            .iter()
            .filter(|old| !next.iter().any(|new| new.id == old.id))
        {
            for (index, &amount) in shelf.snow_depth.iter().enumerate() {
                if amount > 0.0 {
                    released.push((shelf.x + index as f32 + 0.5, shelf.y, amount));
                }
            }
        }
        self.window_shelves = next;
        self.windows = inputs.to_vec();
        for (x, y, amount) in released {
            let radius = (0.7 + amount.sqrt() * 0.2).clamp(0.7, 2.9);
            self.flakes.push(Flake {
                x,
                y: y - radius - amount * 0.5,
                vx: 0.0,
                vy: RELEASED_SNOW_INITIAL_SPEED,
                radius,
                opacity: 0.7,
                phase: self.random.unit() * std::f32::consts::TAU,
                shape: 0,
                snow_mass: Some(amount),
            });
        }
    }

    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.05);
        if self.random.unit() < dt * 0.04 {
            self.target_wind = self.random.signed() * self.wind_strength * 30.0;
        }
        self.wind += (self.target_wind - self.wind) * (1.0 - (-dt * 0.6).exp());
        self.spawn_credit += dt * self.density * (self.width as f32 * 0.18);
        while self.spawn_credit >= 1.0 {
            self.spawn_flake();
            self.spawn_credit -= 1.0;
        }

        let floor_y = self.height as f32;
        let width = self.width;
        let accumulation_speed = self.accumulation_speed;
        let gravity = self.gravity;
        let wind = self.wind;
        for flake in &mut self.flakes {
            let drag = (-dt * 0.18).exp();
            flake.vx = (flake.vx + wind * dt * 0.12) * drag;
            let (gravity_multiplier, terminal_speed) = if flake.snow_mass.is_some() {
                (
                    RELEASED_SNOW_GRAVITY_MULTIPLIER,
                    RELEASED_SNOW_TERMINAL_SPEED,
                )
            } else {
                (14.0, 100.0)
            };
            flake.vy = (flake.vy + gravity * dt * gravity_multiplier).min(terminal_speed);
            flake.phase += dt * (0.6 + flake.radius * 0.08);
            flake.vx += flake.phase.sin() * dt * 5.0;
            flake.x += flake.vx * dt;
            flake.y += flake.vy * dt;
        }

        let mut falling = std::mem::take(&mut self.flakes);
        let mut remaining = std::mem::take(&mut self.flake_scratch);
        remaining.clear();
        let mut deposits = std::mem::take(&mut self.deposit_scratch);
        deposits.clear();
        for mut flake in falling.drain(..) {
            if self.land_on_window(&mut flake, accumulation_speed, dt) {
                continue;
            }
            let index = flake.x.floor().clamp(0.0, width.saturating_sub(1) as f32) as usize;
            if flake.y >= floor_y - self.snow_depth[index] - flake.radius {
                deposits.push((
                    flake.x,
                    flake.snow_mass,
                    flake.radius * 0.12 * accumulation_speed,
                ));
            } else if flake.y < floor_y + 16.0 && flake.x >= -16.0 && flake.x < width as f32 + 16.0
            {
                remaining.push(flake);
            }
        }
        self.flakes = remaining;
        self.flake_scratch = falling;
        for (x, mass, amount) in deposits.drain(..) {
            if let Some(mass) = mass {
                self.deposit_released_on_floor(x, mass);
            } else {
                self.deposit(x - 1.0, amount * 0.25);
                self.deposit(x, amount * 0.5);
                self.deposit(x + 1.0, amount * 0.25);
            }
        }
        self.deposit_scratch = deposits;
        self.settle_surface(dt);
        if let Some(drag) = self.drag.clone()
            && self.snowman_members(drag.index).len() == 1
        {
            let amount = self.gather(drag.pointer_x, drag.pointer_y, 9.0, dt * 5.0);
            self.snowballs[drag.index].mass += amount;
        }
        self.step_snowballs(dt);
    }

    fn step_snowballs(&mut self, dt: f32) {
        let dragged = self.drag.as_ref().map(|drag| drag.index);
        let dragged_members = dragged
            .map(|index| self.snowman_members(index))
            .unwrap_or_default();
        let mut old_positions = std::mem::take(&mut self.snowball_position_scratch);
        old_positions.clear();
        old_positions.extend(self.snowballs.iter().map(|ball| (ball.x, ball.y)));
        for index in 0..self.snowballs.len() {
            if dragged_members.contains(&index)
                || self
                    .snowmen
                    .iter()
                    .any(|man| man.balls.iter().skip(1).any(|&member| member == index))
            {
                continue;
            }
            let ball = &mut self.snowballs[index];
            let radius = ball.radius();
            let old_bottom = ball.y + radius;
            ball.vy += self.gravity * 55.0 * dt;
            let old_x = ball.x;
            ball.x += ball.vx * dt;
            ball.y += ball.vy * dt;
            ball.vx *= (-0.8 * dt).exp();
            let max_x = (self.width as f32 - radius).max(radius);
            let clamped_x = ball.x.clamp(radius.min(max_x), max_x);
            if ball.x != clamped_x {
                ball.vx = 0.0;
                ball.x = clamped_x;
            }
            let mut landing = self.height as f32
                - self.snow_depth[ball
                    .x
                    .floor()
                    .clamp(0.0, self.width.saturating_sub(1) as f32)
                    as usize];
            for window in &self.windows {
                let contact_x = ball.x.clamp(window.x, window.x + window.width - 0.001);
                let dx = (ball.x - contact_x).abs();
                if dx < radius {
                    let local = (contact_x - window.x).floor();
                    let depth = self
                        .window_shelves
                        .iter()
                        .find(|shelf| shelf.id == window.id)
                        .and_then(|shelf| shelf.snow_depth.get(local as usize))
                        .copied()
                        .unwrap_or(0.0);
                    let baseline = rounded_baseline(
                        window.x,
                        window.y,
                        window.width.ceil() as usize,
                        self.window_rounding.min(window.height * 0.5),
                        contact_x,
                    );
                    let top = baseline - depth + radius - (radius * radius - dx * dx).sqrt();
                    if old_bottom <= top && top < landing {
                        landing = top;
                    }
                }
            }
            let landing_time =
                if ball.vy >= 0.0 && old_bottom <= landing && ball.y + radius >= landing {
                    Some((landing - old_bottom) / (ball.y + radius - old_bottom))
                } else {
                    None
                };
            let mut crossed_top = None;
            for window in &self.windows {
                let dy = ball.y + radius - old_bottom;
                if dy <= 0.0 || old_bottom > window.y || ball.y + radius < window.y {
                    continue;
                }
                let t = (window.y - old_bottom) / dy;
                let x = old_x + (ball.x - old_x) * t;
                if x >= window.x
                    && x <= window.x + window.width
                    && landing_time.is_none_or(|old| t < old)
                    && crossed_top.is_none_or(|(old, _)| t < old)
                {
                    crossed_top = Some((t, window.y));
                }
            }
            if let Some((t, top)) = crossed_top {
                ball.x = old_x + (ball.x - old_x) * t;
                ball.y = top - radius;
                ball.vy = 0.0;
                ball.vx *= (-SNOWBALL_GROUND_FRICTION * dt).exp();
            } else if landing_time.is_some() {
                ball.y = landing - radius;
                ball.vy = 0.0;
                ball.vx *= (-SNOWBALL_GROUND_FRICTION * dt).exp();
                if ball.vx.abs() < 3.0 {
                    ball.vx = 0.0;
                }
            } else if ball.y + radius > self.height as f32 {
                ball.y = self.height as f32 - radius;
                ball.vy = 0.0;
                ball.vx *= (-SNOWBALL_GROUND_FRICTION * dt).exp();
            }
        }
        self.align_snowmen(&dragged_members);
        let mut groups = std::mem::take(&mut self.snowball_group_scratch);
        groups.truncate(self.snowballs.len());
        groups.resize_with(self.snowballs.len(), Vec::new);
        for (index, group) in groups.iter_mut().enumerate() {
            group.clear();
            group.push(index);
        }
        for man in &self.snowmen {
            for &index in &man.balls {
                groups[index].clear();
                groups[index].extend_from_slice(&man.balls);
            }
        }
        let mut shattered = std::mem::take(&mut self.shattered_snowball_scratch);
        shattered.clear();
        for (root, group) in groups.iter().enumerate() {
            if group[0] != root || group.iter().any(|index| dragged_members.contains(index)) {
                continue;
            }
            let mut collision = None;
            for &index in group {
                let ball = &self.snowballs[index];
                let (old_x, old_y) = old_positions[index];
                for window in &self.windows {
                    let left = window.x - ball.radius();
                    let right = window.x + window.width + ball.radius();
                    let boundary = if old_x <= left && ball.x > left {
                        Some(left)
                    } else if old_x >= right && ball.x < right {
                        Some(right)
                    } else {
                        None
                    };
                    if let Some(boundary) = boundary {
                        let t = (boundary - old_x) / (ball.x - old_x);
                        let y = old_y + (ball.y - old_y) * t;
                        if y + ball.radius()
                            > window.y + self.window_rounding.min(window.height * 0.5)
                            && y - ball.radius() < window.y + window.height
                        {
                            collision = Some((boundary - ball.x, ball.vx.abs()));
                            break;
                        }
                    }
                }
                if collision.is_some() {
                    break;
                }
            }
            if let Some((correction, speed)) = collision {
                for &index in group {
                    self.snowballs[index].x += correction;
                    if speed < 120.0 {
                        self.snowballs[index].vx = 0.0;
                    }
                }
                if speed >= 120.0 {
                    shattered.extend_from_slice(group);
                }
            }
        }
        if !shattered.is_empty() {
            self.explode_members(&shattered, true);
            self.snowball_position_scratch = old_positions;
            self.snowball_group_scratch = groups;
            self.shattered_snowball_scratch = shattered;
            return;
        }
        for _ in 0..3 {
            for i in 0..self.snowballs.len() {
                for j in i + 1..self.snowballs.len() {
                    if groups[i].contains(&j) {
                        continue;
                    }
                    let a = &self.snowballs[i];
                    let b = &self.snowballs[j];
                    let dx = b.x - a.x;
                    let dy = b.y - a.y;
                    let distance = dx.hypot(dy);
                    let overlap = a.radius() + b.radius() - distance;
                    if overlap <= 0.0 {
                        continue;
                    }
                    let (nx, ny) = if distance > 0.001 {
                        (dx / distance, dy / distance)
                    } else {
                        (0.0, -1.0)
                    };
                    let (move_a, move_b) = if dragged_members.contains(&i) {
                        (0.0, 1.0)
                    } else if dragged_members.contains(&j) {
                        (1.0, 0.0)
                    } else {
                        (0.5, 0.5)
                    };
                    let relative = (b.vx - a.vx) * nx + (b.vy - a.vy) * ny;
                    for &index in &groups[i] {
                        let ball = &mut self.snowballs[index];
                        ball.x -= nx * overlap * move_a;
                        ball.y -= ny * overlap * move_a;
                        if relative < 0.0 {
                            ball.vx += relative * nx * move_a;
                            ball.vy += relative * ny * move_a;
                        }
                    }
                    for &index in &groups[j] {
                        let ball = &mut self.snowballs[index];
                        ball.x += nx * overlap * move_b;
                        ball.y += ny * overlap * move_b;
                        if relative < 0.0 {
                            ball.vx -= relative * nx * move_b;
                            ball.vy -= relative * ny * move_b;
                        }
                    }
                }
            }
            for (index, group) in groups.iter().enumerate() {
                if dragged_members.contains(&index) || group[0] != index {
                    continue;
                }
                let ball = &self.snowballs[index];
                let x = ball
                    .x
                    .floor()
                    .clamp(0.0, self.width.saturating_sub(1) as f32)
                    as usize;
                let floor = self.height as f32 - self.snow_depth[x];
                if ball.y + ball.radius() > floor {
                    let correction = ball.y + ball.radius() - floor;
                    for &member in group {
                        self.snowballs[member].y -= correction;
                        self.snowballs[member].vy = self.snowballs[member].vy.min(0.0);
                    }
                }
            }
        }
        self.align_snowmen(&dragged_members);
        self.assemble_snowmen();
        self.snowball_position_scratch = old_positions;
        self.snowball_group_scratch = groups;
        self.shattered_snowball_scratch = shattered;
    }

    fn align_snowmen(&mut self, dragged: &[usize]) {
        for man in &self.snowmen {
            if man.balls.iter().any(|index| dragged.contains(index)) {
                continue;
            }
            for pair in man.balls.windows(2) {
                let below = &self.snowballs[pair[0]];
                let (x, y, vx, vy) = (
                    below.x,
                    below.y - below.radius() - self.snowballs[pair[1]].radius(),
                    below.vx,
                    below.vy,
                );
                let above = &mut self.snowballs[pair[1]];
                above.x = x;
                above.y = y;
                above.vx = vx;
                above.vy = vy;
            }
        }
    }

    fn assemble_snowmen(&mut self) {
        let dragged = self.drag.as_ref().map(|drag| drag.index);
        let mut used = std::mem::take(&mut self.used_snowball_scratch);
        used.resize(self.snowballs.len(), false);
        used.fill(false);
        for man in &self.snowmen {
            for &index in &man.balls {
                used[index] = true;
            }
        }
        for man in &mut self.snowmen {
            if man.balls.len() >= 3 || dragged.is_some_and(|index| man.balls.contains(&index)) {
                continue;
            }
            let base = man.balls[0];
            let top = *man.balls.last().expect("snowman has balls");
            if let Some(index) = (0..self.snowballs.len()).find(|&index| {
                !used[index]
                    && Some(index) != dragged
                    && (stackable(&self.snowballs[top], &self.snowballs[index])
                        || stackable(&self.snowballs[index], &self.snowballs[base]))
            }) {
                let bottom = self.snowballs[base].y + self.snowballs[base].radius();
                let bottom = bottom.max(self.snowballs[index].y + self.snowballs[index].radius());
                let x = self.snowballs[base].x;
                let vx = self.snowballs[base].vx;
                let vy = self.snowballs[base].vy;
                man.balls.push(index);
                used[index] = true;
                arrange_snowman(&mut self.snowballs, &mut man.balls, x, bottom, vx, vy);
            }
        }
        for lower in 0..self.snowballs.len() {
            if used[lower] || Some(lower) == dragged {
                continue;
            }
            if let Some(upper) = (0..self.snowballs.len()).find(|&upper| {
                upper != lower
                    && !used[upper]
                    && Some(upper) != dragged
                    && stackable(&self.snowballs[lower], &self.snowballs[upper])
            }) {
                used[lower] = true;
                used[upper] = true;
                let mut balls = vec![lower, upper];
                let base = &self.snowballs[lower];
                let (x, bottom, vx, vy) = (base.x, base.y + base.radius(), base.vx, base.vy);
                arrange_snowman(&mut self.snowballs, &mut balls, x, bottom, vx, vy);
                self.snowmen.push(Snowman {
                    balls,
                    red_hat_band: self.random.next() & 1 == 0,
                    red_scarf: self.random.next() & 1 == 0,
                });
            }
        }
        let dragged_members = dragged
            .map(|index| self.snowman_members(index))
            .unwrap_or_default();
        self.align_snowmen(&dragged_members);
        self.used_snowball_scratch = used;
    }

    pub fn height(&self) -> f32 {
        self.height as f32
    }

    fn land_on_window(&mut self, flake: &mut Flake, accumulation_speed: f32, dt: f32) -> bool {
        if self.max_window_snow_depth <= 0.0 {
            return false;
        }
        let previous_bottom = flake.y - flake.vy * dt + flake.radius;
        let next_bottom = flake.y + flake.radius;
        let hit = self
            .window_shelves
            .iter()
            .enumerate()
            .filter_map(|(index, shelf)| {
                let local_x = flake.x - shelf.x;
                if local_x < 0.0 || local_x >= shelf.snow_depth.len() as f32 || flake.vy <= 0.0 {
                    return None;
                }
                let top = shelf.baseline(flake.x) - shelf.snow_depth[local_x.floor() as usize];
                (previous_bottom <= top && next_bottom >= top).then_some((index, top))
            })
            .min_by(|left, right| left.1.total_cmp(&right.1));
        if let Some((shelf_index, _)) = hit {
            let shelf = &mut self.window_shelves[shelf_index];
            let index = (flake.x - shelf.x).floor() as usize;
            if let Some(amount) = flake.snow_mass {
                let baseline = shelf.baseline(flake.x);
                let remaining = deposit_released(&mut shelf.snow_depth, index, amount, |_| {
                    self.max_window_snow_depth
                });
                if remaining > 0.00001 {
                    flake.snow_mass = Some(remaining);
                    flake.y = baseline - flake.radius + 0.001;
                    return false;
                }
                return true;
            }
            let amount = flake.radius * 0.12 * accumulation_speed;
            let last = shelf.snow_depth.len().saturating_sub(1);
            let center = index.min(last);
            shelf.snow_depth[center] =
                (shelf.snow_depth[center] + amount * 0.5).min(self.max_window_snow_depth);
            if center > 0 {
                shelf.snow_depth[center - 1] =
                    (shelf.snow_depth[center - 1] + amount * 0.25).min(self.max_window_snow_depth);
            }
            if center < last {
                shelf.snow_depth[center + 1] =
                    (shelf.snow_depth[center + 1] + amount * 0.25).min(self.max_window_snow_depth);
            }
            return true;
        }
        false
    }

    fn spawn_flake(&mut self) {
        let radius = 0.7 + self.random.unit() * 2.2;
        self.flakes.push(Flake {
            x: self.random.unit() * self.width as f32,
            y: -radius,
            vx: self.random.signed() * 8.0,
            vy: 12.0 + self.random.unit() * 22.0,
            radius,
            opacity: 0.28 + self.random.unit() * 0.6,
            phase: self.random.unit() * std::f32::consts::TAU,
            shape: (self.random.next() % 3) as u8,
            snow_mass: None,
        });
    }

    fn deposit(&mut self, x: f32, amount: f32) {
        let index = x.floor().clamp(0.0, self.width.saturating_sub(1) as f32) as usize;
        self.snow_depth[index] = (self.snow_depth[index] + amount).min(self.max_snow_depth);
    }

    fn deposit_released_on_floor(&mut self, x: f32, amount: f32) {
        let index = x.floor().clamp(0.0, self.width.saturating_sub(1) as f32) as usize;
        let mut remaining =
            deposit_released(&mut self.snow_depth, index, amount, |_| self.max_snow_depth);
        for distance in 5..self.snow_depth.len() {
            if remaining <= 0.00001 {
                break;
            }
            for column in [index.checked_sub(distance), index.checked_add(distance)] {
                let Some(column) = column.filter(|column| *column < self.snow_depth.len()) else {
                    continue;
                };
                let added = remaining.min((self.max_snow_depth - self.snow_depth[column]).max(0.0));
                self.snow_depth[column] += added;
                remaining -= added;
            }
        }
        if remaining > 0.00001 && !self.floor_capacity_warned && self.max_snow_depth > 0.0 {
            eprintln!(
                "christmas-snow: some falling snow could not settle because the bottom snow depth limit was reached"
            );
            self.floor_capacity_warned = true;
        }
    }

    fn settle_surface(&mut self, dt: f32) {
        if self.sliding && self.snow_depth.len() >= 3 {
            self.surface_transfers.resize(self.snow_depth.len(), 0.0);
            self.surface_transfers.fill(0.0);
            for index in 1..self.snow_depth.len() - 1 {
                let left = self.snow_depth[index] - self.snow_depth[index - 1];
                let right = self.snow_depth[index] - self.snow_depth[index + 1];
                let downhill = if left > right { index - 1 } else { index + 1 };
                let slope = left.max(right);
                if slope > 0.8 {
                    let moved = ((slope - 0.8) * dt * 0.025).min(self.snow_depth[index] * 0.2);
                    self.surface_transfers[index] -= moved;
                    self.surface_transfers[downhill] += moved;
                }
            }
            for (depth, transfer) in self.snow_depth.iter_mut().zip(&self.surface_transfers) {
                *depth = (*depth + transfer).clamp(0.0, self.max_snow_depth);
            }
        }
        let compaction = 1.0 - dt * self.compaction * 0.0004;
        if compaction < 1.0 {
            for depth in &mut self.snow_depth {
                *depth *= compaction;
            }
        }
    }
}

fn stackable(lower: &Snowball, upper: &Snowball) -> bool {
    let lower_radius = lower.radius();
    let upper_radius = upper.radius();
    lower_radius >= SNOWMAN_MIN_RADIUS
        && upper_radius >= SNOWMAN_MIN_RADIUS
        && upper.y < lower.y
        && (upper.x - lower.x).abs() <= lower_radius.min(upper_radius) * 0.5
        && ((lower.y - upper.y) - lower_radius - upper_radius).abs() <= 4.0
        && lower.vx.hypot(lower.vy) < 45.0
        && upper.vx.hypot(upper.vy) < 45.0
        && (lower.vx - upper.vx).hypot(lower.vy - upper.vy) < 35.0
}

fn arrange_snowman(
    balls: &mut [Snowball],
    members: &mut [usize],
    x: f32,
    mut bottom: f32,
    vx: f32,
    vy: f32,
) {
    members.sort_by(|&a, &b| balls[b].mass.total_cmp(&balls[a].mass));
    for &index in members.iter() {
        let ball = &mut balls[index];
        ball.x = x;
        ball.y = bottom - ball.radius();
        ball.vx = vx;
        ball.vy = vy;
        bottom = ball.y - ball.radius();
    }
}

fn deposit_released(
    depths: &mut [f32],
    center: usize,
    amount: f32,
    limit: impl Fn(usize) -> f32,
) -> f32 {
    let start = center.saturating_sub(4);
    let end = (center + 5).min(depths.len());
    let mut remaining = amount;
    for (index, depth) in depths.iter_mut().enumerate().take(end).skip(start) {
        let columns_left = (end - index) as f32;
        let available = (limit(index) - *depth).max(0.0);
        let added = (remaining / columns_left).min(available);
        *depth += added;
        remaining -= added;
    }
    for (index, depth) in depths.iter_mut().enumerate().take(end).skip(start) {
        if remaining <= 0.00001 {
            break;
        }
        let added = remaining.min((limit(index) - *depth).max(0.0));
        *depth += added;
        remaining -= added;
    }
    remaining
}

fn resample(values: &[f32], size: usize) -> Vec<f32> {
    if values.is_empty() || size == 0 {
        return vec![0.0; size];
    }
    if size == 1 {
        return vec![values[0]];
    }
    let scale = (values.len() - 1) as f32 / (size - 1) as f32;
    (0..size)
        .map(|index| {
            let source = index as f32 * scale;
            let left = source.floor() as usize;
            let right = (left + 1).min(values.len() - 1);
            values[left] + (values[right] - values[left]) * (source - left as f32)
        })
        .collect()
}

#[derive(Clone, Debug)]
struct Random(u64);

impl Random {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1_u32 << 24) as f32
    }

    fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_shelf_geometry_matches_window_radius() {
        let mut sim = Simulation::new(200, 200, 0.0, 0.0, 20.0);
        sim.set_window_rounding(10.0);
        sim.set_windows(&[window("a", 30.0, 80.0, 100.0, 80.0)], true);
        let shelf = &sim.window_shelves[0];
        assert_eq!(shelf.baseline(30.0), 90.0);
        assert_eq!(shelf.baseline(40.0), 80.0);
        assert_eq!(shelf.baseline(130.0), 90.0);
        assert!(shelf.baseline(35.0) > 80.0);
        sim.set_window_rounding(0.0);
        assert_eq!(sim.window_shelves[0].baseline(30.0), 80.0);
    }

    #[test]
    fn releasing_a_snowman_inside_a_window_falls_without_teleporting() {
        let mut sim = Simulation::new(400, 400, 0.0, 0.0, 20.0);
        sim.set_windows(&[window("a", 100.0, 180.0, 200.0, 100.0)], true);
        sim.snowballs = vec![ball(200.0, 130.0, 20.0), ball(200.0, 90.0, 20.0)];
        sim.assemble_snowmen();
        assert!(sim.begin_drag(200.0, 130.0));
        sim.move_drag(200.0, 165.0, 0.1);
        sim.stop_drag_motion();
        sim.end_drag();
        assert_eq!(sim.snowballs[0].y, 165.0);
        assert_eq!(sim.snowballs[1].y, 125.0);
        sim.step(1.0 / 60.0);
        assert!(sim.snowballs[0].y > 165.0 && sim.snowballs[0].y < 166.0);
        assert!((sim.snowballs[0].y - sim.snowballs[1].y - 40.0).abs() < 0.01);
        for _ in 0..60 {
            sim.step(1.0 / 60.0);
        }
        assert!((sim.snowballs[0].y - 380.0).abs() < 0.01);
    }

    #[test]
    fn releasing_a_ball_inside_a_window_preserves_position_and_velocity() {
        let mut sim = Simulation::new(500, 500, 0.0, 0.0, 20.0);
        sim.set_windows(&[window("a", 100.0, 100.0, 300.0, 250.0)], true);
        sim.snowballs.push(ball(200.0, 200.0, 20.0));
        assert!(sim.begin_drag(200.0, 200.0));
        sim.move_drag(220.0, 210.0, 0.1);
        sim.end_drag();
        assert_eq!((sim.snowballs[0].x, sim.snowballs[0].y), (220.0, 210.0));
        assert_eq!((sim.snowballs[0].vx, sim.snowballs[0].vy), (200.0, 100.0));
        sim.step(0.01);
        assert!(sim.snowballs[0].x > 220.0);
        assert!(sim.snowballs[0].y > 210.0 && sim.snowballs[0].y < 212.0);
    }

    #[test]
    fn fast_side_impact_shatters_and_conserves_snow_and_slow_impact_stops() {
        for (speed, shatters) in [(240.0, true), (50.0, false), (-240.0, true), (-50.0, false)] {
            let mut sim = Simulation::new(500, 300, 0.0, 0.0, 200.0);
            sim.set_gravity(0.0);
            sim.set_windows(&[window("a", 200.0, 50.0, 100.0, 150.0)], true);
            let x = if speed > 0.0 { 180.0 } else { 320.0 };
            sim.snowballs.push(Snowball {
                vx: speed,
                ..ball(x, 100.0, 20.0)
            });
            let mass = sim.snowballs[0].mass;
            sim.step(0.05);
            if shatters {
                assert!(sim.snowballs.is_empty());
                assert!(
                    (sim.flakes.iter().map(|f| f.snow_mass.unwrap()).sum::<f32>() - mass).abs()
                        < 0.01
                );
                sim.set_gravity(9.81);
                for _ in 0..300 {
                    sim.step(1.0 / 60.0);
                }
                assert!(sim.snow_depth.iter().sum::<f32>() > 100.0);
            } else {
                assert_eq!(sim.snowballs.len(), 1);
                assert_eq!(sim.snowballs[0].vx, 0.0);
                assert!((sim.snowballs[0].x - x).abs() < 0.01);
            }
        }
    }

    #[test]
    fn exploding_snowman_preserves_mass_and_other_snowman_indices() {
        let mut sim = Simulation::new(600, 400, 0.0, 0.0, 100.0);
        sim.snowballs = vec![
            ball(100.0, 380.0, 20.0),
            ball(100.0, 340.0, 20.0),
            ball(400.0, 380.0, 20.0),
            ball(400.0, 340.0, 20.0),
        ];
        sim.assemble_snowmen();
        let mass = sim.snowballs[0].mass + sim.snowballs[1].mass;
        let generation = sim.ball_generation();
        let members = sim.snowman_members(1);
        sim.explode_members(&members, true);
        assert_eq!(sim.ball_generation(), generation + 1);
        assert_eq!(sim.snowmen.len(), 1);
        assert_eq!(sim.snowmen[0].balls, vec![0, 1]);
        assert!((sim.flakes.iter().map(|f| f.snow_mass.unwrap()).sum::<f32>() - mass).abs() < 0.01);
        for _ in 0..300 {
            sim.step(1.0 / 60.0);
        }

        assert!(sim.snow_depth.iter().sum::<f32>() > 100.0);
    }

    #[test]
    fn double_click_explosion_does_not_accumulate_snow() {
        let mut sim = Simulation::new(600, 400, 0.0, 0.0, 100.0);
        sim.set_windows(&[window("a", 100.0, 330.0, 400.0, 50.0)], true);
        sim.snowballs = vec![ball(300.0, 300.0, 20.0), ball(300.0, 260.0, 20.0)];
        sim.assemble_snowmen();

        sim.explode_ball_without_snow(1);

        assert!(sim.snowballs.is_empty());
        assert!(sim.snowmen.is_empty());
        assert!(!sim.flakes.is_empty());
        assert!(sim.flakes.iter().all(|flake| flake.snow_mass == Some(0.0)));
        for _ in 0..300 {
            sim.step(1.0 / 60.0);
        }

        assert!(sim.snow_depth.iter().all(|depth| *depth == 0.0));
        assert!(
            sim.window_shelves
                .iter()
                .flat_map(|shelf| &shelf.snow_depth)
                .all(|depth| *depth == 0.0)
        );
    }

    #[test]
    fn side_impact_on_snowman_shatters_the_whole_group() {
        let mut sim = Simulation::new(500, 300, 0.0, 0.0, 100.0);
        sim.set_gravity(0.0);
        sim.set_windows(&[window("a", 200.0, 30.0, 100.0, 200.0)], true);
        sim.snowballs = vec![ball(180.0, 130.0, 20.0), ball(180.0, 90.0, 20.0)];
        sim.assemble_snowmen();
        let mass = sim.snowballs.iter().map(|b| b.mass).sum::<f32>();
        for ball in &mut sim.snowballs {
            ball.vx = 240.0;
        }
        sim.step(0.05);
        assert!(sim.snowballs.is_empty());
        assert!(sim.snowmen.is_empty());
        assert!((sim.flakes.iter().map(|f| f.snow_mass.unwrap()).sum::<f32>() - mass).abs() < 0.01);
    }

    #[test]
    fn swept_window_top_catches_fast_snowman() {
        let mut sim = Simulation::new(400, 400, 0.0, 0.0, 20.0);
        sim.set_windows(&[window("a", 100.0, 180.0, 200.0, 100.0)], true);
        sim.snowballs = vec![ball(200.0, 120.0, 20.0), ball(200.0, 80.0, 20.0)];
        sim.assemble_snowmen();
        for ball in &mut sim.snowballs {
            ball.vy = 600.0;
        }
        for _ in 0..120 {
            sim.step(0.05);
        }
        assert!((sim.snowballs[0].y - 160.0).abs() < 0.01);
        assert!((sim.snowballs[1].y - 120.0).abs() < 0.01);
    }

    fn ball(x: f32, y: f32, radius: f32) -> Snowball {
        Snowball {
            x,
            y,
            vx: 0.0,
            vy: 0.0,
            mass: std::f32::consts::PI * radius * radius,
        }
    }

    #[test]
    fn shovel_clears_snow_along_a_sweep_on_floor_and_window_shelves() {
        let mut sim = Simulation::new(200, 200, 0.0, 0.0, 30.0);
        sim.set_windows(&[window("a", 40.0, 80.0, 100.0, 50.0)], true);
        sim.snow_depth.fill(12.0);
        sim.window_shelves[0].snow_depth.fill(8.0);

        sim.shovel_snow((50.0, 195.0), (100.0, 195.0), 6.0);
        sim.shovel_snow((70.0, 77.0), (70.0, 77.0), 6.0);

        assert!(sim.snow_depth[55..96].iter().all(|depth| *depth == 0.0));
        assert_eq!(sim.snow_depth[40], 12.0);
        assert!(
            sim.window_shelves[0].snow_depth[24..36]
                .iter()
                .all(|depth| *depth == 0.0)
        );
        assert_eq!(sim.window_shelves[0].snow_depth[10], 8.0);
    }

    #[test]
    fn clearing_accumulated_snow_preserves_falling_flakes_and_snowballs() {
        let mut sim = Simulation::new(200, 200, 0.0, 0.0, 30.0);
        sim.set_windows(&[window("a", 40.0, 80.0, 100.0, 50.0)], true);
        sim.snow_depth.fill(12.0);
        sim.window_shelves[0].snow_depth.fill(8.0);
        sim.snowballs.push(ball(100.0, 100.0, 12.0));
        sim.flakes.push(Flake {
            x: 20.0,
            y: 30.0,
            vx: 0.0,
            vy: 1.0,
            radius: 1.0,
            opacity: 1.0,
            phase: 0.0,
            shape: 0,
            snow_mass: None,
        });

        sim.clear_accumulated_snow();

        assert!(sim.snow_depth.iter().all(|depth| *depth == 0.0));
        assert!(
            sim.window_shelves[0]
                .snow_depth
                .iter()
                .all(|depth| *depth == 0.0)
        );
        assert_eq!(sim.snowballs.len(), 1);
        assert_eq!(sim.flakes.len(), 1);
    }

    #[test]
    fn snowmen_form_from_settled_stacks_and_accept_a_third_ball() {
        let mut sim = Simulation::new(300, 300, 0.0, 0.0, 30.0);
        sim.snowballs = vec![ball(100.0, 280.0, 20.0), ball(100.0, 240.0, 20.0)];
        for _ in 0..60 {
            sim.step(1.0 / 60.0);
        }
        assert_eq!(sim.snowmen.len(), 1);
        assert_eq!(sim.snowmen[0].balls, vec![0, 1]);
        let colors = (sim.snowmen[0].red_hat_band, sim.snowmen[0].red_scarf);
        sim.snowballs.push(ball(100.0, 200.0, 20.0));
        for _ in 0..60 {
            sim.step(1.0 / 60.0);
        }
        assert_eq!(sim.snowmen[0].balls, vec![0, 1, 2]);
        assert_eq!(
            colors,
            (sim.snowmen[0].red_hat_band, sim.snowmen[0].red_scarf)
        );
        for pair in sim.snowmen[0].balls.windows(2) {
            let a = &sim.snowballs[pair[0]];
            let b = &sim.snowballs[pair[1]];
            assert!((a.y - b.y - a.radius() - b.radius()).abs() < 0.001);
        }
    }

    #[test]
    fn snowmen_reorder_all_size_permutations_and_keep_their_base_in_place() {
        for radii in [
            [12.0, 18.0, 24.0],
            [12.0, 24.0, 18.0],
            [18.0, 12.0, 24.0],
            [18.0, 24.0, 12.0],
            [24.0, 12.0, 18.0],
            [24.0, 18.0, 12.0],
        ] {
            let mut sim = Simulation::new(400, 400, 0.0, 0.0, 30.0);
            sim.snowballs = vec![
                ball(200.0, 400.0 - radii[0], radii[0]),
                ball(200.0, 400.0 - radii[0] * 2.0 - radii[1], radii[1]),
            ];
            sim.assemble_snowmen();
            assert_eq!(sim.snowmen.len(), 1);
            let top = *sim.snowmen[0].balls.last().unwrap();
            let top_y = sim.snowballs[top].y - sim.snowballs[top].radius();
            sim.snowballs.push(ball(200.0, top_y - radii[2], radii[2]));
            for _ in 0..120 {
                sim.step(1.0 / 60.0);
            }
            assert_eq!(sim.snowmen[0].balls.len(), 3, "{radii:?}");
            let ordered: Vec<_> = sim.snowmen[0]
                .balls
                .iter()
                .map(|&i| sim.snowballs[i].radius())
                .collect();
            assert_eq!(ordered, vec![24.0, 18.0, 12.0], "{radii:?}");
            let base = &sim.snowballs[sim.snowmen[0].balls[0]];
            assert!((base.y + base.radius() - 400.0).abs() < 0.01);
            for pair in sim.snowmen[0].balls.windows(2) {
                let a = &sim.snowballs[pair[0]];
                let b = &sim.snowballs[pair[1]];
                assert!((a.y - b.y - a.radius() - b.radius()).abs() < 0.01);
            }
        }
    }

    #[test]
    fn adding_ball_below_existing_snowman_reorders_and_keeps_bottom() {
        let mut sim = Simulation::new(400, 400, 0.0, 0.0, 30.0);
        sim.snowballs = vec![ball(200.0, 300.0, 12.0), ball(200.0, 270.0, 18.0)];
        sim.assemble_snowmen();
        let base = sim.snowmen[0].balls[0];
        let bottom = sim.snowballs[base].y + sim.snowballs[base].radius();
        sim.snowballs.push(ball(200.0, bottom + 24.0, 24.0));
        sim.assemble_snowmen();
        assert_eq!(sim.snowmen[0].balls, vec![2, 1, 0]);
        assert!((sim.snowballs[2].y + sim.snowballs[2].radius() - (bottom + 48.0)).abs() < 0.01);
    }

    #[test]
    fn larger_ball_dropped_onto_smaller_one_forms_sorted_snowman() {
        let mut sim = Simulation::new(400, 400, 0.0, 0.0, 30.0);
        sim.snowballs = vec![ball(200.0, 388.0, 12.0), ball(200.0, 260.0, 24.0)];
        for _ in 0..300 {
            sim.step(1.0 / 60.0);
        }
        assert_eq!(sim.snowmen.len(), 1);
        assert_eq!(sim.snowmen[0].balls, vec![1, 0]);
        assert!((sim.snowballs[1].y + 24.0 - 400.0).abs() < 0.01);
    }

    #[test]
    fn snowman_stops_gliding_on_a_window_shelf() {
        let mut sim = Simulation::new(600, 400, 0.0, 0.0, 30.0);
        sim.set_windows(&[window("ledge", 0.0, 300.0, 600.0, 80.0)], true);
        sim.snowballs = vec![ball(200.0, 280.0, 20.0), ball(200.0, 240.0, 20.0)];
        sim.assemble_snowmen();
        sim.snowballs[0].vx = 200.0;
        sim.snowballs[1].vx = 200.0;
        for _ in 0..120 {
            sim.step(1.0 / 60.0);
        }
        assert!(sim.snowballs[0].x - 200.0 < 35.0);
        assert_eq!(sim.snowballs[0].vx, 0.0);
        assert!((sim.snowballs[0].y - 280.0).abs() < 0.01);
    }

    #[test]
    fn throw_inherits_pointer_velocity_and_follows_gravity() {
        let mut sim = Simulation::new(3000, 2000, 0.0, 0.0, 30.0);
        sim.snowballs.push(ball(400.0, 800.0, 20.0));
        assert!(sim.begin_drag(400.0, 800.0));
        sim.move_drag(440.0, 780.0, 0.1);
        sim.end_drag();
        assert_eq!((sim.snowballs[0].vx, sim.snowballs[0].vy), (400.0, -200.0));
        let release_x = sim.snowballs[0].x;
        let release_y = sim.snowballs[0].y;
        sim.step(1.0 / 60.0);
        assert!(sim.snowballs[0].x > release_x);
        assert!(sim.snowballs[0].y < release_y);
        assert!(sim.snowballs[0].vy > -200.0);
        let mut highest = sim.snowballs[0].y;
        for _ in 0..60 {
            sim.step(1.0 / 60.0);
            highest = highest.min(sim.snowballs[0].y);
        }
        assert!(release_y - highest > 20.0);
        assert!(sim.snowballs[0].y > release_y);
        assert!(sim.snowballs[0].x - release_x > 40.0);
    }

    #[test]
    fn throw_velocity_is_not_capped_or_scaled() {
        let mut sim = Simulation::new(10000, 10000, 0.0, 0.0, 30.0);
        sim.snowballs.push(ball(2000.0, 2000.0, 20.0));
        assert!(sim.begin_drag(2000.0, 2000.0));
        sim.move_drag(2120.0, 1940.0, 0.1);
        sim.end_drag();
        assert!((sim.snowballs[0].vx - 1200.0).abs() < 0.01);
        assert!((sim.snowballs[0].vy + 600.0).abs() < 0.01);
        sim.step(0.01);
        assert!(sim.snowballs[0].vy < -590.0);
    }

    #[test]
    fn fast_diagonal_throw_crossing_narrow_window_top_is_caught() {
        let mut sim = Simulation::new(500, 500, 0.0, 0.0, 30.0);
        sim.set_windows(&[window("ledge", 100.0, 200.0, 8.0, 100.0)], false);
        sim.snowballs.push(Snowball {
            vx: 2000.0,
            vy: 1500.0,
            ..ball(50.0, 150.0, 10.0)
        });
        sim.step(0.05);
        let ball = &sim.snowballs[0];
        assert!(ball.x >= 90.0 && ball.x < 130.0, "x={}", ball.x);
        assert!((ball.y - 190.0).abs() < 0.01, "y={}", ball.y);
    }

    #[test]
    fn snow_friction_stops_gliding() {
        let mut sim = Simulation::new(1000, 1000, 0.0, 0.0, 30.0);
        sim.snowballs.push(ball(400.0, 980.0, 20.0));
        sim.snowballs[0] = Snowball {
            vx: 200.0,
            ..ball(400.0, 980.0, 20.0)
        };
        for _ in 0..120 {
            sim.step(1.0 / 60.0);
        }
        assert!(sim.snowballs[0].x - 400.0 < 35.0);
        assert_eq!(sim.snowballs[0].vx, 0.0);
    }

    #[test]
    fn paused_or_unmeasurable_drag_does_not_throw() {
        let mut sim = Simulation::new(500, 500, 0.0, 0.0, 30.0);
        sim.snowballs.push(ball(100.0, 100.0, 20.0));
        assert!(sim.begin_drag(100.0, 100.0));
        sim.move_drag(140.0, 100.0, 0.1);
        sim.move_drag(150.0, 100.0, 0.0);
        sim.end_drag();
        assert_eq!(sim.snowballs[0].vx, 0.0);
    }

    #[test]
    fn snowman_release_preserves_pointer_velocity_for_every_member() {
        let mut sim = Simulation::new(1000, 1000, 0.0, 0.0, 30.0);
        sim.snowballs = vec![ball(200.0, 500.0, 20.0), ball(200.0, 460.0, 20.0)];
        sim.assemble_snowmen();
        assert!(sim.begin_drag(200.0, 460.0));
        sim.move_drag(250.0, 440.0, 0.1);
        sim.end_drag();
        for ball in &sim.snowballs {
            assert_eq!((ball.vx, ball.vy), (500.0, -200.0));
        }
        sim.step(0.05);
        assert!((sim.snowballs[0].y - sim.snowballs[1].y - 40.0).abs() < 0.001);
        assert!(sim.snowballs[0].x > 250.0);
    }

    #[test]
    fn configured_zero_gravity_remains_zero() {
        let mut sim = Simulation::new(400, 400, 0.0, 0.0, 30.0);
        sim.set_gravity(0.0);
        sim.snowballs.push(ball(100.0, 100.0, 20.0));
        for _ in 0..60 {
            sim.step(1.0 / 60.0);
        }
        assert_eq!(sim.snowballs[0].y, 100.0);
    }

    #[test]
    fn snowman_formation_rejects_small_sideways_fast_and_held_balls() {
        let lower = ball(100.0, 200.0, 20.0);
        assert!(!stackable(&lower, &ball(100.0, 171.0, 9.0)));
        assert!(!stackable(&lower, &ball(115.0, 163.0, 20.0)));
        assert!(!stackable(
            &lower,
            &Snowball {
                vy: 100.0,
                ..ball(100.0, 160.0, 20.0)
            }
        ));
        let mut sim = Simulation::new(300, 300, 0.0, 0.0, 30.0);
        sim.snowballs = vec![lower, ball(100.0, 160.0, 20.0)];
        assert!(sim.begin_drag(100.0, 160.0));
        sim.assemble_snowmen();
        assert!(sim.snowmen.is_empty());
        sim.end_drag();
        sim.assemble_snowmen();
        assert_eq!(sim.snowmen.len(), 1);
    }

    #[test]
    fn dragging_any_member_moves_the_whole_snowman_and_it_lands_together() {
        let mut sim = Simulation::new(400, 400, 0.0, 0.0, 30.0);
        sim.snowballs = vec![ball(100.0, 360.0, 20.0), ball(100.0, 320.0, 20.0)];
        sim.assemble_snowmen();
        assert!(sim.begin_drag(105.0, 320.0));
        sim.move_drag(155.0, 200.0, 0.1);
        assert_eq!((sim.snowballs[0].x, sim.snowballs[1].x), (150.0, 150.0));
        assert_eq!((sim.snowballs[0].y, sim.snowballs[1].y), (240.0, 200.0));
        sim.stop_drag_motion();
        sim.end_drag();
        for _ in 0..180 {
            sim.step(1.0 / 60.0);
        }
        assert!((sim.snowballs[0].y - 380.0).abs() < 0.01);
        assert!((sim.snowballs[1].y - 340.0).abs() < 0.01);
    }

    #[test]
    fn snowman_members_do_not_split_when_hit_by_another_ball() {
        let mut sim = Simulation::new(400, 400, 0.0, 0.0, 30.0);
        sim.snowballs = vec![ball(100.0, 380.0, 20.0), ball(100.0, 340.0, 20.0)];
        sim.assemble_snowmen();
        sim.snowballs.push(ball(135.0, 340.0, 20.0));
        for _ in 0..10 {
            sim.step(1.0 / 60.0);
        }
        assert_eq!(sim.snowmen.len(), 1);
        assert!((sim.snowballs[0].x - sim.snowballs[1].x).abs() < 0.001);
        assert!((sim.snowballs[0].y - sim.snowballs[1].y - 40.0).abs() < 0.001);
        assert!(
            (sim.snowballs[2].x - sim.snowballs[1].x)
                .hypot(sim.snowballs[2].y - sim.snowballs[1].y)
                > 39.0
        );
    }

    #[test]
    fn snowman_lands_on_window_and_separate_stacks_stay_separate() {
        let mut sim = Simulation::new(400, 400, 0.0, 0.0, 30.0);
        sim.set_windows(&[window("ledge", 50.0, 250.0, 110.0, 80.0)], true);
        sim.snowballs = vec![
            ball(100.0, 150.0, 20.0),
            ball(100.0, 110.0, 20.0),
            ball(300.0, 380.0, 20.0),
            ball(300.0, 340.0, 20.0),
        ];
        sim.assemble_snowmen();
        assert_eq!(sim.snowmen.len(), 2);
        for _ in 0..180 {
            sim.step(1.0 / 60.0);
        }
        assert_eq!(sim.snowmen[0].balls, vec![0, 1]);
        assert_eq!(sim.snowmen[1].balls, vec![2, 3]);
        assert!((sim.snowballs[0].y - 230.0).abs() < 0.01);
        assert!((sim.snowballs[1].y - 190.0).abs() < 0.01);
    }

    #[test]
    fn dragging_snow_creates_and_grows_a_ball_with_mass_taken_from_surface() {
        let mut sim = Simulation::new(200, 200, 0.0, 0.0, 50.0);
        sim.snow_depth.fill(12.0);
        let before: f32 = sim.snow_depth.iter().sum();
        assert!(sim.begin_drag(50.0, 190.0));
        let initial_mass = sim.snowballs[0].mass;
        sim.move_drag(80.0, 190.0, 0.1);
        let after: f32 = sim.snow_depth.iter().sum();
        assert!(sim.snowballs[0].mass > initial_mass);
        assert!((before - after - sim.snowballs[0].mass).abs() < 0.01);
        sim.end_drag();
        for _ in 0..30 {
            sim.step(1.0 / 60.0);
        }
        assert!(sim.snowballs[0].y + sim.snowballs[0].radius() <= 200.0);
    }

    #[test]
    fn drag_preserves_grab_offset_and_release_velocity() {
        let mut sim = Simulation::new(200, 200, 0.0, 0.0, 20.0);
        sim.snowballs.push(Snowball {
            x: 40.0,
            y: 40.0,
            vx: 0.0,
            vy: 0.0,
            mass: std::f32::consts::PI * 100.0,
        });
        assert!(sim.begin_drag(45.0, 40.0));
        sim.move_drag(65.0, 55.0, 0.1);
        assert_eq!((sim.snowballs[0].x, sim.snowballs[0].y), (60.0, 55.0));
        assert_eq!((sim.snowballs[0].vx, sim.snowballs[0].vy), (200.0, 150.0));
        sim.end_drag();
        sim.step(0.05);
        assert!(sim.snowballs[0].x > 60.0);
    }

    #[test]
    fn snowballs_settle_in_a_stack() {
        let mut sim = Simulation::new(200, 200, 0.0, 0.0, 20.0);
        let mass = std::f32::consts::PI * 100.0;
        sim.snowballs.push(Snowball {
            x: 100.0,
            y: 190.0,
            vx: 0.0,
            vy: 0.0,
            mass,
        });
        sim.snowballs.push(Snowball {
            x: 100.0,
            y: 130.0,
            vx: 0.0,
            vy: 0.0,
            mass,
        });
        for _ in 0..600 {
            sim.step(1.0 / 60.0);
        }
        assert!((sim.snowballs[0].y - 190.0).abs() < 1.0);
        assert!((sim.snowballs[1].y - 170.0).abs() < 2.0);
    }

    #[test]
    fn snowball_lands_on_window_without_window_accumulation() {
        let mut sim = Simulation::new(200, 200, 0.0, 0.0, 20.0);
        sim.set_windows(&[window("a", 30.0, 100.0, 100.0, 20.0)], false);
        sim.snowballs.push(Snowball {
            x: 80.0,
            y: 40.0,
            vx: 0.0,
            vy: 0.0,
            mass: std::f32::consts::PI * 100.0,
        });
        for _ in 0..120 {
            sim.step(1.0 / 60.0);
        }
        assert!((sim.snowballs[0].y - 90.0).abs() < 0.1);
    }
    #[test]
    fn flakes_spawn_with_varied_properties_and_fall() {
        let mut simulation = Simulation::new(1000, 500, 1.0, 0.4, 100.0);
        for _ in 0..120 {
            simulation.step(1.0 / 60.0);
        }
        assert!(simulation.flakes.len() > 100);
        assert!(simulation.flakes.iter().any(|flake| flake.y > 0.0));
        assert_ne!(simulation.flakes[0].radius, simulation.flakes[1].radius);
    }

    #[test]
    fn landed_flakes_accumulate_and_stay_under_the_depth_limit() {
        let mut simulation = Simulation::new(80, 10, 1.0, 0.0, 4.0);
        for _ in 0..30_000 {
            simulation.step(1.0 / 60.0);
        }
        assert!(simulation.snow_depth.iter().any(|depth| *depth > 0.0));
        assert!(simulation.snow_depth.iter().all(|depth| *depth <= 4.0));
    }

    #[test]
    fn invalid_time_steps_do_not_corrupt_the_simulation() {
        let mut simulation = Simulation::new(20, 20, 0.5, 0.5, 20.0);
        simulation.step(f32::NAN);
        simulation.step(-1.0);
        assert!(simulation.flakes.is_empty());
    }

    #[test]
    fn settled_snow_compacts_over_time() {
        let mut simulation = Simulation::new(8, 8, 0.0, 0.0, 40.0);
        simulation.snow_depth.fill(20.0);
        for _ in 0..1200 {
            simulation.step(1.0 / 60.0);
        }
        assert!(simulation.snow_depth.iter().all(|depth| *depth < 20.0));
    }

    #[test]
    fn flakes_accumulate_on_window_edges_and_follow_geometry() {
        let mut simulation = Simulation::new(40, 40, 0.0, 0.0, 20.0);
        simulation.set_windows(
            &[WindowShelfInput {
                id: "window-a".to_owned(),
                x: 10.0,
                y: 12.0,
                width: 10.0,
                height: 10.0,
            }],
            true,
        );
        simulation.flakes.push(Flake {
            x: 14.0,
            y: 10.0,
            vx: 0.0,
            vy: 100.0,
            radius: 2.0,
            opacity: 1.0,
            phase: 0.0,
            shape: 0,
            snow_mass: None,
        });
        simulation.step(0.05);
        assert!(simulation.flakes.is_empty());
        assert!(
            simulation.window_shelves[0]
                .snow_depth
                .iter()
                .any(|depth| *depth > 0.0)
        );

        let prior_depth = simulation.window_shelves[0].snow_depth.clone();
        simulation.set_windows(
            &[WindowShelfInput {
                id: "window-a".to_owned(),
                x: 18.0,
                y: 22.0,
                width: 20.0,
                height: 10.0,
            }],
            true,
        );
        assert_eq!(simulation.window_shelves[0].x, 18.0);
        assert!(
            simulation.window_shelves[0]
                .snow_depth
                .iter()
                .all(|depth| *depth == 0.0)
        );
        assert!(!simulation.flakes.is_empty());
        assert_ne!(
            prior_depth.len(),
            simulation.window_shelves[0].snow_depth.len()
        );
    }

    #[test]
    fn window_snow_respects_its_independent_depth_limit() {
        let mut simulation = Simulation::new(40, 40, 0.0, 0.0, 4.0);
        simulation.set_window_snow_depth_limit(0.5);
        simulation.set_windows(
            &[WindowShelfInput {
                id: "window-a".to_owned(),
                x: 10.0,
                y: 12.0,
                width: 10.0,
                height: 10.0,
            }],
            true,
        );
        for _ in 0..400 {
            simulation.flakes.push(Flake {
                x: 14.0,
                y: 10.0,
                vx: 0.0,
                vy: 100.0,
                radius: 2.0,
                opacity: 1.0,
                phase: 0.0,
                shape: 0,
                snow_mass: None,
            });
            simulation.step(0.05);
        }
        assert!(
            simulation.window_shelves[0]
                .snow_depth
                .iter()
                .all(|depth| *depth <= 0.5)
        );
        assert!(
            simulation.window_shelves[0]
                .snow_depth
                .iter()
                .any(|depth| *depth > 0.0)
        );
        assert!(simulation.snow_depth.iter().all(|depth| *depth <= 4.0));
    }

    fn window(id: &str, x: f32, y: f32, width: f32, height: f32) -> WindowShelfInput {
        WindowShelfInput {
            id: id.to_owned(),
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn released_snow_falls_faster_than_individual_flakes() {
        let mut simulation = Simulation::new(200, 2000, 0.0, 0.0, 20.0);
        simulation.set_windows(&[window("a", 30.0, 100.0, 20.0, 20.0)], true);
        simulation.window_shelves[0].snow_depth[5] = 2.0;
        simulation.set_windows(&[], true);
        assert_eq!(simulation.flakes.len(), 1);
        let released = simulation.flakes[0];
        assert!(released.vy > 100.0);
        let initial_y = released.y;
        simulation.flakes.push(Flake {
            x: 100.0,
            y: released.y,
            vy: 34.0,
            snow_mass: None,
            ..released
        });
        for _ in 0..60 {
            simulation.step(1.0 / 60.0);
        }
        let released = simulation
            .flakes
            .iter()
            .find(|f| f.snow_mass.is_some())
            .unwrap();
        let normal = simulation
            .flakes
            .iter()
            .find(|f| f.snow_mass.is_none())
            .unwrap();
        assert!(released.vy > normal.vy * 2.0);
        assert!(released.y - initial_y > (normal.y - initial_y) * 1.5);
    }

    #[test]
    fn fast_released_snow_collides_with_a_window_without_tunneling() {
        let mut simulation = Simulation::new(100, 500, 0.0, 0.0, 20.0);
        simulation.set_windows(&[window("a", 20.0, 100.0, 40.0, 20.0)], true);
        simulation.window_shelves[0].snow_depth[10] = 2.0;
        simulation.set_windows(&[window("a", 20.0, 130.0, 40.0, 20.0)], true);
        for _ in 0..10 {
            simulation.step(0.05);
        }
        assert!(simulation.flakes.is_empty());
        assert!((simulation.window_shelves[0].snow_depth.iter().sum::<f32>() - 2.0).abs() < 0.001);
        assert!(simulation.snow_depth.iter().all(|depth| *depth == 0.0));
    }

    #[test]
    fn closed_window_snow_settles_on_lower_window_with_visible_width() {
        let mut sim = Simulation::new(120, 250, 0.0, 0.0, 20.0);
        sim.set_sliding(false);
        sim.set_compaction(0.0);
        sim.set_windows(
            &[
                window("upper", 20.0, 50.0, 60.0, 20.0),
                window("lower", 20.0, 140.0, 60.0, 20.0),
            ],
            true,
        );
        sim.window_shelves
            .iter_mut()
            .find(|s| s.id == "upper")
            .unwrap()
            .snow_depth[20] = 9.0;
        sim.set_windows(&[window("lower", 20.0, 140.0, 60.0, 20.0)], true);
        for _ in 0..90 {
            sim.step(1.0 / 60.0);
        }
        let lower = &sim.window_shelves[0].snow_depth;
        assert!((lower.iter().sum::<f32>() - 9.0).abs() < 0.001);
        assert!(lower.iter().filter(|depth| **depth > 0.01).count() >= 5);
        assert!(sim.flakes.is_empty());
        assert_eq!(sim.snow_depth.iter().sum::<f32>(), 0.0);
    }

    #[test]
    fn released_snow_settles_across_multiple_floor_columns() {
        let mut sim = Simulation::new(120, 200, 0.0, 0.0, 20.0);
        sim.set_sliding(false);
        sim.set_compaction(0.0);
        sim.set_windows(&[window("upper", 20.0, 50.0, 60.0, 20.0)], true);
        sim.window_shelves[0].snow_depth[20] = 9.0;
        sim.set_windows(&[], true);
        for _ in 0..120 {
            sim.step(1.0 / 60.0);
        }
        assert!((sim.snow_depth.iter().sum::<f32>() - 9.0).abs() < 0.001);
        assert!(sim.snow_depth.iter().filter(|depth| **depth > 0.01).count() >= 5);
        assert!(sim.flakes.is_empty());
    }

    #[test]
    fn full_window_shelf_passes_falling_snow_to_lower_window() {
        let mut sim = Simulation::new(120, 250, 0.0, 0.0, 20.0);
        sim.set_sliding(false);
        sim.set_compaction(0.0);
        sim.set_window_snow_depth_limit(4.0);
        sim.set_windows(
            &[
                window("upper", 20.0, 100.0, 60.0, 20.0),
                window("lower", 20.0, 160.0, 60.0, 20.0),
            ],
            true,
        );
        sim.window_shelves
            .iter_mut()
            .find(|s| s.id == "upper")
            .unwrap()
            .snow_depth
            .fill(4.0);
        sim.flakes.push(Flake {
            x: 45.0,
            y: 70.0,
            vx: 0.0,
            vy: 200.0,
            radius: 1.0,
            opacity: 1.0,
            phase: 0.0,
            shape: 0,
            snow_mass: Some(3.0),
        });
        for _ in 0..90 {
            sim.step(1.0 / 60.0);
        }
        let lower = &sim
            .window_shelves
            .iter()
            .find(|s| s.id == "lower")
            .unwrap()
            .snow_depth;
        assert!((lower.iter().sum::<f32>() - 3.0).abs() < 0.001);
        assert!(sim.flakes.is_empty());
    }

    #[test]
    fn full_local_floor_redistributes_snow_to_free_columns() {
        let mut sim = Simulation::new(30, 200, 0.0, 0.0, 2.0);
        sim.snow_depth[6..15].fill(2.0);
        sim.deposit_released_on_floor(10.0, 3.0);
        assert!((sim.snow_depth.iter().sum::<f32>() - 21.0).abs() < 0.001);
        assert!(sim.snow_depth.iter().all(|depth| *depth <= 2.0));
    }

    #[test]
    fn moving_window_does_not_destroy_falling_accumulated_snow() {
        let mut sim = Simulation::new(100, 200, 0.0, 0.0, 20.0);
        sim.set_sliding(false);
        sim.set_compaction(0.0);
        sim.set_windows(&[window("a", 20.0, 130.0, 50.0, 30.0)], true);
        sim.flakes.push(Flake {
            x: 40.0,
            y: 100.0,
            vx: 0.0,
            vy: 150.0,
            radius: 1.0,
            opacity: 1.0,
            phase: 0.0,
            shape: 0,
            snow_mass: Some(3.0),
        });
        sim.set_windows(&[window("a", 20.0, 90.0, 50.0, 30.0)], true);
        assert_eq!(sim.flakes.len(), 1);
        for _ in 0..50 {
            sim.step(1.0 / 60.0);
        }
        assert!((sim.snow_depth.iter().sum::<f32>() - 3.0).abs() < 0.001);
    }

    #[test]
    fn sideways_movement_sheds_unsupported_snow_in_both_directions() {
        for offset in [-5.0, 5.0] {
            let mut simulation = Simulation::new(100, 100, 0.0, 0.0, 20.0);
            simulation.set_windows(&[window("a", 30.0, 20.0, 20.0, 20.0)], true);
            simulation.window_shelves[0].snow_depth.fill(2.0);
            let new_x = 30.0 + offset;
            simulation.set_windows(&[window("a", new_x, 20.0, 20.0, 20.0)], true);
            let retained: f32 = simulation.window_shelves[0].snow_depth.iter().sum();
            let released: f32 = simulation
                .flakes
                .iter()
                .map(|flake| flake.snow_mass.unwrap())
                .sum();
            assert_eq!(retained, 30.0);
            assert_eq!(released, 10.0);
            assert!(
                simulation
                    .flakes
                    .iter()
                    .all(|flake| { flake.x < new_x || flake.x >= new_x + 20.0 })
            );
            for _ in 0..120 {
                simulation.step(1.0 / 60.0);
            }
            assert!(simulation.snow_depth.iter().any(|depth| *depth > 0.0));
        }
    }

    #[test]
    fn downward_movement_releases_snow_then_it_lands_back_on_the_window() {
        let mut simulation = Simulation::new(100, 100, 0.0, 0.0, 20.0);
        simulation.set_windows(&[window("a", 20.0, 20.0, 40.0, 20.0)], true);
        simulation.window_shelves[0].snow_depth[10..30].fill(2.0);
        simulation.set_windows(&[window("a", 20.0, 35.0, 40.0, 20.0)], true);
        assert!(
            simulation.window_shelves[0]
                .snow_depth
                .iter()
                .all(|depth| *depth == 0.0)
        );
        assert_eq!(
            simulation
                .flakes
                .iter()
                .map(|flake| flake.snow_mass.unwrap())
                .sum::<f32>(),
            40.0
        );
        assert!(
            simulation
                .flakes
                .iter()
                .all(|flake| flake.y + flake.radius < 35.0)
        );
        for _ in 0..120 {
            simulation.step(1.0 / 60.0);
        }
        assert!(simulation.flakes.is_empty());
        assert!((simulation.window_shelves[0].snow_depth.iter().sum::<f32>() - 40.0).abs() < 0.001);
    }

    #[test]
    fn upward_movement_and_stationary_resize_keep_snow_attached() {
        let mut simulation = Simulation::new(100, 100, 0.0, 0.0, 20.0);
        simulation.set_windows(&[window("a", 20.0, 30.0, 20.0, 20.0)], true);
        simulation.window_shelves[0].snow_depth.fill(2.0);
        simulation.set_windows(&[window("a", 20.0, 20.0, 20.0, 20.0)], true);
        assert!(simulation.flakes.is_empty());
        assert!(
            simulation.window_shelves[0]
                .snow_depth
                .iter()
                .all(|depth| *depth == 2.0)
        );
        simulation.set_windows(&[window("a", 20.0, 20.0, 30.0, 20.0)], true);
        assert!(simulation.flakes.is_empty());
        assert!(
            simulation.window_shelves[0]
                .snow_depth
                .iter()
                .all(|depth| *depth == 2.0)
        );
    }

    #[test]
    fn snow_can_reland_after_repeated_downward_geometry_updates() {
        let mut simulation = Simulation::new(120, 120, 0.0, 0.0, 20.0);
        simulation.set_windows(&[window("a", 20.0, 20.0, 60.0, 20.0)], true);
        simulation.window_shelves[0].snow_depth[20..40].fill(2.0);
        for offset in 1..=15 {
            simulation.set_windows(&[window("a", 20.0, 20.0 + offset as f32, 60.0, 20.0)], true);
            simulation.step(0.05);
        }
        for _ in 0..120 {
            simulation.step(1.0 / 60.0);
        }
        assert!(simulation.flakes.is_empty());
        assert!((simulation.window_shelves[0].snow_depth.iter().sum::<f32>() - 40.0).abs() < 0.001);
    }

    #[test]
    fn new_and_moved_windows_preserve_overlapping_floor_snow() {
        let mut simulation = Simulation::new(40, 40, 0.0, 0.0, 30.0);
        simulation.snow_depth.fill(12.0);
        simulation.set_windows(&[window("a", 5.0, 20.0, 10.0, 18.0)], true);
        assert_eq!(simulation.snow_depth[4], 12.0);
        assert_eq!(simulation.snow_depth[5], 12.0);
        assert_eq!(simulation.snow_depth[14], 12.0);
        assert_eq!(simulation.snow_depth[15], 12.0);
        simulation.set_windows(&[window("a", 20.0, 20.0, 10.0, 20.0)], true);
        assert_eq!(simulation.snow_depth[5], 12.0);
        assert_eq!(simulation.snow_depth[20], 12.0);
        simulation.set_compaction(0.0);
        simulation.step(0.05);
        assert_eq!(simulation.snow_depth[20], 12.0);
    }

    #[test]
    fn overlapping_window_preserves_shelf_and_close_releases_snow() {
        let mut simulation = Simulation::new(50, 50, 0.0, 0.0, 30.0);
        let base = window("base", 10.0, 30.0, 20.0, 10.0);
        simulation.set_windows(std::slice::from_ref(&base), true);
        simulation.window_shelves[0].snow_depth.fill(6.0);
        let cover = window("cover", 15.0, 20.0, 5.0, 15.0);
        simulation.set_windows(&[base.clone(), cover], true);
        let shelf = simulation
            .window_shelves
            .iter()
            .find(|s| s.id == "base")
            .unwrap();
        assert_eq!(shelf.snow_depth[0], 6.0);
        assert_eq!(shelf.snow_depth[5], 6.0);
        assert_eq!(shelf.snow_depth[10], 6.0);
        simulation.set_windows(&[], true);
        assert!(simulation.window_shelves.is_empty());
        assert!(!simulation.flakes.is_empty());
        assert!(simulation.snow_depth.iter().all(|depth| *depth == 0.0));
    }

    #[test]
    fn released_snow_can_settle_under_an_overlapping_window() {
        let mut simulation = Simulation::new(100, 100, 0.0, 0.0, 30.0);
        simulation.set_sliding(false);
        simulation.set_compaction(0.0);
        simulation.set_windows(&[window("cover", 20.0, 50.0, 50.0, 50.0)], true);
        simulation.deposit_released_on_floor(40.0, 9.0);
        assert!((simulation.snow_depth.iter().sum::<f32>() - 9.0).abs() < 0.001);
        assert!(simulation.snow_depth[40] > 0.0);
        simulation.step(0.05);
        assert!((simulation.snow_depth.iter().sum::<f32>() - 9.0).abs() < 0.001);
    }

    #[test]
    fn released_snow_accumulates_on_shelf_overlapped_by_another_window() {
        let mut sim = Simulation::new(100, 200, 0.0, 0.0, 30.0);
        sim.set_compaction(0.0);
        sim.set_windows(
            &[
                window("cover", 10.0, 30.0, 70.0, 100.0),
                window("base", 10.0, 120.0, 70.0, 20.0),
            ],
            true,
        );
        let mut flake = Flake {
            x: 40.0,
            y: 120.0,
            vx: 0.0,
            vy: 150.0,
            radius: 1.0,
            opacity: 1.0,
            phase: 0.0,
            shape: 0,
            snow_mass: Some(5.0),
        };
        assert!(sim.land_on_window(&mut flake, 1.0, 0.05));
        let shelf = sim.window_shelves.iter().find(|s| s.id == "base").unwrap();
        assert!((shelf.snow_depth.iter().sum::<f32>() - 5.0).abs() < 0.001);
        sim.step(0.05);
        assert!(
            (sim.window_shelves
                .iter()
                .find(|s| s.id == "base")
                .unwrap()
                .snow_depth
                .iter()
                .sum::<f32>()
                - 5.0)
                .abs()
                < 0.001
        );
    }

    #[test]
    fn disabling_window_accumulation_preserves_covered_floor() {
        let mut simulation = Simulation::new(30, 30, 0.0, 0.0, 20.0);
        simulation.snow_depth.fill(10.0);
        simulation.set_windows(&[window("a", 2.0, 15.0, 10.0, 15.0)], false);
        assert!(simulation.window_shelves.is_empty());
        assert_eq!(simulation.snow_depth[5], 10.0);
        assert_eq!(simulation.snow_depth[15], 10.0);
    }

    #[test]
    fn moving_windows_preserves_particles_inside_windows() {
        let mut simulation = Simulation::new(30, 30, 0.0, 0.0, 20.0);
        simulation.set_windows(&[window("a", 2.0, 10.0, 10.0, 15.0)], true);
        let flake = Flake {
            x: 5.0,
            y: 15.0,
            vx: 0.0,
            vy: 10.0,
            radius: 1.0,
            opacity: 1.0,
            phase: 0.0,
            shape: 0,
            snow_mass: None,
        };
        simulation.flakes.push(flake);
        simulation.flakes.push(Flake { x: 20.0, ..flake });
        simulation.set_windows(&[window("a", 3.0, 10.0, 10.0, 15.0)], true);
        assert_eq!(simulation.flakes, vec![flake, Flake { x: 20.0, ..flake }]);
    }

    #[test]
    fn collision_uses_topmost_intersected_shelf_with_overlapping_widths() {
        let mut simulation = Simulation::new(60, 60, 0.0, 0.0, 20.0);
        simulation.set_windows(
            &[
                window("wide", 0.0, 20.0, 50.0, 5.0),
                window("narrow", 5.0, 35.0, 5.0, 5.0),
                window("right", 15.0, 40.0, 30.0, 5.0),
            ],
            true,
        );
        let mut flake = Flake {
            x: 25.0,
            y: 22.0,
            vx: 0.0,
            vy: 100.0,
            radius: 1.0,
            opacity: 1.0,
            phase: 0.0,
            shape: 0,
            snow_mass: None,
        };
        assert!(simulation.land_on_window(&mut flake, 1.0, 0.05));
        assert!(
            simulation
                .window_shelves
                .iter()
                .find(|s| s.id == "wide")
                .unwrap()
                .snow_depth
                .iter()
                .any(|d| *d > 0.0)
        );
        assert!(
            simulation
                .window_shelves
                .iter()
                .find(|s| s.id == "right")
                .unwrap()
                .snow_depth
                .iter()
                .all(|d| *d == 0.0)
        );
    }
}
