use crate::{config::Settings, sim::Simulation};
use std::collections::HashMap;

pub struct Workspaces {
    simulations: HashMap<i64, Simulation>,
    monitors: HashMap<i64, String>,
    background: HashMap<String, Simulation>,
    settings: Settings,
}

fn new_simulation(settings: &Settings, width: u32, height: u32) -> Simulation {
    let mut simulation = Simulation::new(
        width,
        height,
        settings.snow.density,
        settings.snow.wind_strength,
        if settings.snow.accumulation {
            settings.snow.max_snow_depth
        } else {
            0.0
        },
    );
    simulation.set_accumulation_speed(settings.snow.accumulation_speed);
    simulation.set_window_snow_depth_limit(if settings.windows.accumulate_on_windows {
        settings.windows.max_snow_depth
    } else {
        0.0
    });
    simulation.set_particle_size(settings.snow.particle_size);
    if settings.physics.enabled {
        simulation.set_gravity(settings.physics.gravity);
        simulation.set_compaction(settings.physics.snow_compaction);
        simulation.set_sliding(settings.physics.sliding);
    } else {
        simulation.set_gravity(0.0);
        simulation.set_compaction(0.0);
        simulation.set_sliding(false);
    }
    simulation
}

impl Workspaces {
    pub fn new(settings: &Settings) -> Self {
        Self {
            simulations: HashMap::new(),
            monitors: HashMap::new(),
            background: HashMap::new(),
            settings: settings.clone(),
        }
    }

    pub fn get(&mut self, workspace: i64, width: u32, height: u32) -> &mut Simulation {
        let simulation = self
            .simulations
            .entry(workspace)
            .or_insert_with(|| new_simulation(&self.settings, width, height));
        simulation.resize(width, height);
        simulation
    }

    pub fn activate(&mut self, workspace: i64, monitor: &str, width: u32, height: u32) {
        let background = self
            .background
            .entry(monitor.to_owned())
            .or_insert_with(|| new_simulation(&self.settings, width, height));
        background.resize(width, height);
        self.simulations
            .entry(workspace)
            .or_insert_with(|| {
                let mut simulation = new_simulation(&self.settings, width, height);
                simulation.snow_depth.clone_from(&background.snow_depth);
                simulation
            })
            .resize(width, height);
        self.monitors.insert(workspace, monitor.to_owned());
    }

    pub fn step_monitor(&mut self, monitor: &str, dt: f32) {
        if let Some(background) = self.background.get_mut(monitor) {
            background.step(dt);
        }
        for (workspace, simulation) in &mut self.simulations {
            if self
                .monitors
                .get(workspace)
                .is_some_and(|name| name == monitor)
            {
                simulation.step(dt);
            }
        }
    }

    pub fn clear_accumulated_snow(&mut self) {
        for simulation in self.simulations.values_mut() {
            simulation.clear_accumulated_snow();
        }
        for simulation in self.background.values_mut() {
            simulation.clear_accumulated_snow();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Snowball, WindowShelfInput};

    #[test]
    fn inactive_and_new_workspaces_accumulate_without_losing_independent_state() {
        let mut workspaces = Workspaces::new(&Settings::default());
        workspaces.activate(1, "monitor", 200, 120);
        workspaces.activate(2, "monitor", 200, 120);
        for _ in 0..1800 {
            workspaces.step_monitor("monitor", 1.0 / 60.0);
        }
        let first = workspaces.get(1, 200, 120).snow_depth.iter().sum::<f32>();
        let second = workspaces.get(2, 200, 120).snow_depth.iter().sum::<f32>();
        assert!(first > 1.0, "snow should visibly accumulate: {first}");
        assert!((first - second).abs() < 0.01);
        workspaces.get(2, 200, 120).snow_depth[50] = 10.0;
        workspaces.activate(3, "monitor", 200, 120);
        let third = workspaces.get(3, 200, 120).snow_depth.iter().sum::<f32>();
        assert!((first - third).abs() < 0.01);
        assert_eq!(workspaces.get(2, 200, 120).snow_depth[50], 10.0);
    }

    #[test]
    fn inactive_window_shelves_accumulate_and_monitors_advance_independently() {
        let mut workspaces = Workspaces::new(&Settings::default());
        workspaces.activate(1, "one", 200, 120);
        workspaces.activate(2, "one", 200, 120);
        workspaces.activate(3, "two", 200, 120);
        workspaces.get(2, 200, 120).set_windows(
            &[WindowShelfInput {
                id: "window".to_owned(),
                x: 0.0,
                y: 80.0,
                width: 200.0,
                height: 30.0,
            }],
            true,
        );
        for _ in 0..1800 {
            workspaces.step_monitor("one", 1.0 / 60.0);
        }
        let shelf = &workspaces.get(2, 200, 120).window_shelves[0].snow_depth;
        assert!(shelf.iter().sum::<f32>() > 1.0);
        assert_eq!(
            workspaces.get(3, 200, 120).snow_depth.iter().sum::<f32>(),
            0.0
        );
    }

    #[test]
    fn snowmen_remain_on_their_own_workspace() {
        let mut workspaces = Workspaces::new(&Settings::default());
        let first = workspaces.get(1, 200, 200);
        for y in [180.0, 140.0] {
            first.snowballs.push(Snowball {
                x: 100.0,
                y,
                vx: 0.0,
                vy: 0.0,
                mass: std::f32::consts::PI * 400.0,
            });
        }
        first.step(1.0 / 60.0);
        assert_eq!(first.snowmen.len(), 1);
        assert!(workspaces.get(2, 200, 200).snowmen.is_empty());
        assert_eq!(workspaces.get(1, 200, 200).snowmen[0].balls, vec![0, 1]);
    }

    #[test]
    fn workspaces_restore_independent_snow_after_switching_and_resize() {
        let mut workspaces = Workspaces::new(&Settings::default());
        workspaces.get(1, 10, 10).snow_depth[3] = 5.0;
        assert_eq!(workspaces.get(2, 10, 10).snow_depth[3], 0.0);
        workspaces.get(2, 10, 10).snow_depth[3] = 2.0;
        assert_eq!(workspaces.get(1, 12, 10).snow_depth[3], 5.0);
        assert_eq!(workspaces.get(2, 10, 10).snow_depth[3], 2.0);
    }

    #[test]
    fn window_snow_is_restored_and_reconciled_after_return() {
        let mut workspaces = Workspaces::new(&Settings::default());
        let window = WindowShelfInput {
            id: "window-a".to_owned(),
            x: 2.0,
            y: 10.0,
            width: 10.0,
            height: 10.0,
        };
        let first = workspaces.get(1, 40, 40);
        first.set_windows(std::slice::from_ref(&window), true);
        first.window_shelves[0].snow_depth.fill(3.0);
        let second = workspaces.get(2, 40, 40);
        second.set_windows(&[], true);
        assert!(second.window_shelves.is_empty());

        let first = workspaces.get(1, 40, 40);
        first.set_windows(std::slice::from_ref(&window), true);
        assert_eq!(first.window_shelves[0].snow_depth[3], 3.0);
        let moved = WindowShelfInput {
            x: 15.0,
            width: 20.0,
            ..window
        };
        first.set_windows(&[moved], true);
        assert_eq!(first.window_shelves[0].x, 15.0);
        assert!(
            first.window_shelves[0]
                .snow_depth
                .iter()
                .all(|depth| *depth == 0.0)
        );
        assert!(!first.flakes.is_empty());
        first.set_windows(&[], true);
        assert!(first.window_shelves.is_empty());
        assert!(!first.flakes.is_empty());
    }

    #[test]
    fn clearing_accumulation_clears_all_workspaces_and_backgrounds() {
        let mut workspaces = Workspaces::new(&Settings::default());
        workspaces.activate(1, "monitor-a", 200, 120);
        workspaces.activate(2, "monitor-a", 200, 120);
        workspaces.activate(3, "monitor-b", 200, 120);
        workspaces.get(1, 200, 120).snow_depth.fill(8.0);
        workspaces.get(2, 200, 120).snow_depth.fill(9.0);
        workspaces.get(3, 200, 120).snow_depth.fill(10.0);
        let window = WindowShelfInput {
            id: "window".to_owned(),
            x: 20.0,
            y: 40.0,
            width: 80.0,
            height: 40.0,
        };
        for workspace in [1, 2, 3] {
            let simulation = workspaces.get(workspace, 200, 120);
            simulation.set_windows(std::slice::from_ref(&window), true);
            simulation.window_shelves[0].snow_depth.fill(5.0);
        }

        workspaces.clear_accumulated_snow();

        assert!(workspaces.simulations.values().all(|simulation| {
            simulation.snow_depth.iter().all(|depth| *depth == 0.0)
                && simulation
                    .window_shelves
                    .iter()
                    .all(|shelf| shelf.snow_depth.iter().all(|depth| *depth == 0.0))
        }));
        assert!(
            workspaces
                .background
                .values()
                .all(|simulation| simulation.snow_depth.iter().all(|depth| *depth == 0.0))
        );
    }
}
