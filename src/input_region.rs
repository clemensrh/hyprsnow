use crate::sim::Simulation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub fn full_surface(width: u32, height: u32) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: width.min(i32::MAX as u32) as i32,
        height: height.min(i32::MAX as u32) as i32,
    }
}

#[cfg(test)]
pub fn regions(simulation: &Simulation, scale: i32, width: u32, height: u32) -> Vec<Rect> {
    let mut rectangles = Vec::new();
    regions_into(&mut rectangles, simulation, scale, width, height, false);
    rectangles
}

pub fn regions_into(
    rectangles: &mut Vec<Rect>,
    simulation: &Simulation,
    scale: i32,
    width: u32,
    height: u32,
    shovel_active: bool,
) {
    rectangles.clear();
    if shovel_active {
        rectangles.push(full_surface(width, height));
        return;
    }
    let scale = scale.max(1) as f32;
    let mut add_surface = |depths: &[f32], origin: f32, baseline: &dyn Fn(f32) -> f32| {
        let start = (origin / scale).floor().max(0.0) as i32;
        let end = ((origin + depths.len() as f32) / scale)
            .ceil()
            .min(width as f32) as i32;
        for x in start..end {
            let physical = ((x as f32 + 0.5) * scale - origin).floor() as isize;
            if physical < 0 || physical as usize >= depths.len() {
                continue;
            }
            let depth = depths[physical as usize];
            let baseline = baseline((x as f32 + 0.5) * scale);
            let top = ((baseline - depth) / scale).ceil().max(0.0) as i32;
            let bottom = (baseline / scale).floor().min(height as f32) as i32;
            if bottom > top {
                if let Some(last) = rectangles.last_mut()
                    && last.x + last.width == x
                    && last.y == top
                    && last.height == bottom - top
                {
                    last.width += 1;
                } else {
                    rectangles.push(Rect {
                        x,
                        y: top,
                        width: 1,
                        height: bottom - top,
                    });
                }
            }
        }
    };
    add_surface(&simulation.snow_depth, 0.0, &|_| simulation.height());
    for shelf in &simulation.window_shelves {
        add_surface(&shelf.snow_depth, shelf.x, &|x| shelf.baseline(x));
    }
    for ball in &simulation.snowballs {
        let x = ball.x / scale;
        let y = ball.y / scale;
        let radius = ball.radius() / scale;
        let top = (y - radius).ceil().max(0.0) as i32;
        let bottom = (y + radius).floor().min(height as f32) as i32;
        for row in top..bottom {
            let dy = (row as f32 + 0.5 - y).abs();
            let half = (radius * radius - dy * dy).max(0.0).sqrt();
            let left = (x - half).ceil().max(0.0) as i32;
            let right = (x + half).floor().min(width as f32) as i32;
            if right > left {
                rectangles.push(Rect {
                    x: left,
                    y: row,
                    width: right - left,
                    height: 1,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Snowball;

    #[test]
    fn input_region_covers_only_visible_snow_and_balls() {
        let mut sim = Simulation::new(20, 20, 0.0, 0.0, 20.0);
        assert!(regions(&sim, 1, 20, 20).is_empty());
        sim.snow_depth[5] = 3.0;
        let rects = regions(&sim, 1, 20, 20);
        assert_eq!(
            rects,
            vec![Rect {
                x: 5,
                y: 17,
                width: 1,
                height: 3
            }]
        );
    }

    #[test]
    fn input_region_buffer_reuses_capacity_and_clears_stale_rectangles() {
        let mut sim = Simulation::new(20, 20, 0.0, 0.0, 20.0);
        sim.snow_depth[5] = 3.0;
        let mut rectangles = Vec::new();

        regions_into(&mut rectangles, &sim, 1, 20, 20, false);
        let capacity = rectangles.capacity();
        assert_eq!(rectangles.len(), 1);

        sim.snow_depth.fill(0.0);
        regions_into(&mut rectangles, &sim, 1, 20, 20, false);

        assert!(rectangles.is_empty());
        assert_eq!(rectangles.capacity(), capacity);
    }

    #[test]
    fn shovel_drag_keeps_input_over_cleared_snow_until_release() {
        let mut sim = Simulation::new(40, 40, 0.0, 0.0, 20.0);
        sim.snow_depth[10..20].fill(6.0);
        let mut rectangles = Vec::new();
        regions_into(&mut rectangles, &sim, 2, 20, 20, true);
        assert_eq!(
            rectangles,
            vec![Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 20,
            }]
        );
        sim.shovel_snow((15.0, 38.0), (15.0, 38.0), 15.0);
        assert!(sim.snow_depth.iter().all(|depth| *depth == 0.0));
        regions_into(&mut rectangles, &sim, 2, 20, 20, true);
        assert_eq!(rectangles[0].width, 20);
        assert_eq!(rectangles[0].height, 20);
        regions_into(&mut rectangles, &sim, 2, 20, 20, false);
        assert!(rectangles.is_empty());
    }

    #[test]
    fn shovel_on_empty_workspace_keeps_input_through_press_motion_and_release() {
        let mut sim = Simulation::new(100, 40, 0.0, 0.0, 20.0);
        sim.snow_depth[10..90].fill(6.0);
        assert!(sim.window_shelves.is_empty());
        let mut rectangles = Vec::new();

        regions_into(&mut rectangles, &sim, 1, 100, 40, false);
        assert!(rectangles.iter().any(|rect| rect.x <= 15
            && rect.x + rect.width > 15
            && rect.y <= 38
            && rect.y + rect.height > 38));

        regions_into(&mut rectangles, &sim, 1, 100, 40, true);
        assert_eq!(rectangles, vec![full_surface(100, 40)]);
        sim.shovel_snow((15.0, 38.0), (15.0, 38.0), 15.0);
        sim.shovel_snow((15.0, 38.0), (70.0, 38.0), 15.0);
        assert!(sim.snow_depth[15..70].iter().all(|depth| *depth == 0.0));

        regions_into(&mut rectangles, &sim, 1, 100, 40, true);
        assert_eq!(rectangles, vec![full_surface(100, 40)]);
        regions_into(&mut rectangles, &sim, 1, 100, 40, false);
        assert!(!rectangles.iter().any(|rect| rect.x <= 70
            && rect.x + rect.width > 70
            && rect.y <= 38
            && rect.y + rect.height > 38));
    }

    #[test]
    fn scaled_regions_do_not_cover_empty_surface() {
        let mut sim = Simulation::new(40, 40, 0.0, 0.0, 20.0);
        sim.snow_depth[10..20].fill(6.0);
        let rects = regions(&sim, 2, 20, 20);
        assert_eq!(
            rects,
            vec![Rect {
                x: 5,
                y: 17,
                width: 5,
                height: 3
            }]
        );
    }

    #[test]
    fn rounded_shelf_input_tracks_visible_snow() {
        let mut sim = Simulation::new(200, 200, 0.0, 0.0, 20.0);
        sim.set_window_rounding(10.0);
        sim.set_windows(
            &[crate::sim::WindowShelfInput {
                id: "window".to_owned(),
                x: 20.0,
                y: 80.0,
                width: 100.0,
                height: 50.0,
            }],
            true,
        );
        sim.window_shelves[0].snow_depth.fill(4.0);
        let region = regions(&sim, 1, 200, 200);
        let covers = |x, y| {
            region
                .iter()
                .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
        };
        assert!(covers(20, 85));
        assert!(!covers(20, 77));
        assert!(covers(30, 77));
    }

    #[test]
    fn snowball_region_covers_center_but_not_its_bounding_box_corners() {
        let mut sim = Simulation::new(100, 100, 0.0, 0.0, 20.0);
        sim.snowballs.push(Snowball {
            x: 50.0,
            y: 50.0,
            vx: 0.0,
            vy: 0.0,
            mass: std::f32::consts::PI * 100.0,
        });
        let regions = regions(&sim, 1, 100, 100);
        let covers = |x, y| {
            regions
                .iter()
                .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
        };
        assert!(covers(50, 50));
        assert!(!covers(40, 40));
        assert!(!covers(30, 50));
    }
}
