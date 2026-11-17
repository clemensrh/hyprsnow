use crate::config::{self, Settings, SnowOption};
use ksni::blocking::TrayMethods;
use std::{
    process::Command,
    sync::mpsc::{self, RecvTimeoutError, Sender},
    time::Duration,
};

const DENSITY_PRESETS: [(f32, &str); 4] = [
    (0.25, "Light (0.25)"),
    (0.5, "Moderate (0.5)"),
    (0.7, "Default (0.7)"),
    (1.0, "Heavy (1.0)"),
];
const WIND_PRESETS: [(f32, &str); 4] = [
    (0.0, "Calm (0.0)"),
    (0.25, "Breezy (0.25)"),
    (0.5, "Windy (0.5)"),
    (1.0, "Strong (1.0)"),
];
const PARTICLE_SIZE_PRESETS: [(f32, &str); 4] = [
    (0.5, "Small (0.5)"),
    (1.0, "Normal (1.0)"),
    (1.5, "Large (1.5)"),
    (2.0, "Extra large (2.0)"),
];

enum Action {
    ToggleService,
    ToggleSnow,
    ToggleAccumulation,
    SetDensity(f32),
    SetWindStrength(f32),
    SetParticleSize(f32),
    ClearAccumulatedSnow,
    OpenConfig,
    Exit,
}

struct SnowTray {
    settings: Settings,
    service_active: bool,
    actions: Sender<Action>,
}

impl SnowTray {
    fn send(&self, action: Action) {
        if let Err(error) = self.actions.send(action) {
            eprintln!("christmas-snow tray: cannot queue menu action: {error}");
        }
    }
}

impl ksni::Tray for SnowTray {
    fn id(&self) -> String {
        "christmas-snow".to_owned()
    }

    fn title(&self) -> String {
        "Christmas Snow".to_owned()
    }

    fn icon_name(&self) -> String {
        "weather-snow".to_owned()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![snowflake_icon()]
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;

        vec![
            CheckmarkItem {
                label: "Overlay service running".to_owned(),
                checked: self.service_active,
                activate: Box::new(|tray: &mut Self| tray.send(Action::ToggleService)),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Snowfall enabled".to_owned(),
                checked: self.settings.general.enabled && self.settings.snow.enabled,
                activate: Box::new(|tray: &mut Self| tray.send(Action::ToggleSnow)),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Snow accumulation".to_owned(),
                checked: self.settings.snow.accumulation,
                activate: Box::new(|tray: &mut Self| tray.send(Action::ToggleAccumulation)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            preset_submenu(
                "Snow density",
                self.settings.snow.density,
                &DENSITY_PRESETS,
                Action::SetDensity,
            ),
            preset_submenu(
                "Wind",
                self.settings.snow.wind_strength,
                &WIND_PRESETS,
                Action::SetWindStrength,
            ),
            preset_submenu(
                "Particle size",
                self.settings.snow.particle_size,
                &PARTICLE_SIZE_PRESETS,
                Action::SetParticleSize,
            ),
            MenuItem::Separator,
            StandardItem {
                label: "Clear accumulated snow".to_owned(),
                icon_name: "edit-clear".to_owned(),
                activate: Box::new(|tray: &mut Self| tray.send(Action::ClearAccumulatedSnow)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open configuration".to_owned(),
                icon_name: "document-edit".to_owned(),
                activate: Box::new(|tray: &mut Self| tray.send(Action::OpenConfig)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit tray icon".to_owned(),
                icon_name: "application-exit".to_owned(),
                activate: Box::new(|tray: &mut Self| tray.send(Action::Exit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn preset_submenu(
    label: &str,
    current: f32,
    presets: &'static [(f32, &'static str)],
    action: fn(f32) -> Action,
) -> ksni::MenuItem<SnowTray> {
    use ksni::menu::*;

    SubMenu {
        label: label.to_owned(),
        submenu: vec![
            RadioGroup {
                selected: closest_preset(current, presets),
                select: Box::new(move |tray: &mut SnowTray, selected| {
                    if let Some((value, _)) = presets.get(selected) {
                        tray.send(action(*value));
                    }
                }),
                options: presets
                    .iter()
                    .map(|(_, label)| RadioItem {
                        label: (*label).to_owned(),
                        ..Default::default()
                    })
                    .collect(),
            }
            .into(),
        ],
        ..Default::default()
    }
    .into()
}

fn closest_preset(current: f32, presets: &[(f32, &str)]) -> usize {
    presets
        .iter()
        .enumerate()
        .min_by(|(_, (left, _)), (_, (right, _))| {
            (current - left).abs().total_cmp(&(current - right).abs())
        })
        .map_or(0, |(index, _)| index)
}

pub fn run(settings: Settings) -> Result<(), String> {
    let (sender, receiver) = mpsc::channel();
    let service_active = crate::service_is_active()?;
    let tray = SnowTray {
        settings,
        service_active,
        actions: sender,
    };
    let handle = tray
        .assume_sni_available(true)
        .spawn()
        .map_err(|error| format!("cannot start system tray icon: {error}"))?;

    loop {
        match receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(Action::Exit) => return Ok(()),
            Ok(action) => {
                if let Err(error) = apply_action(action, &handle) {
                    eprintln!("christmas-snow tray: {error}");
                }
                refresh_tray(&handle);
            }
            Err(RecvTimeoutError::Timeout) => refresh_tray(&handle),
            Err(RecvTimeoutError::Disconnected) => {
                return Err("tray menu event channel disconnected".to_owned());
            }
        }
    }
}

fn apply_action(action: Action, handle: &ksni::blocking::Handle<SnowTray>) -> Result<(), String> {
    match action {
        Action::ToggleService => {
            if crate::service_is_active()? {
                crate::systemd_action("stop")
            } else {
                crate::systemd_action("start")
            }
        }
        Action::ToggleSnow => {
            let settings = Settings::load()?;
            let enabled = !(settings.general.enabled && settings.snow.enabled);
            let updated = config::update_snow_option(SnowOption::Enabled(enabled))?;
            let action = if updated.snow.enabled && updated.general.enabled {
                if crate::service_is_active()? {
                    "restart"
                } else {
                    "start"
                }
            } else if crate::service_is_active()? {
                "stop"
            } else {
                return Ok(());
            };
            handle
                .update(|tray| tray.settings = updated)
                .ok_or("tray service is no longer available")?;
            crate::systemd_action(action)
        }
        Action::ToggleAccumulation => update_setting(SnowOption::Accumulation(
            !Settings::load()?.snow.accumulation,
        ))
        .and_then(restart_if_active),
        Action::SetDensity(value) => {
            update_setting(SnowOption::Density(value)).and_then(restart_if_active)
        }
        Action::SetWindStrength(value) => {
            update_setting(SnowOption::WindStrength(value)).and_then(restart_if_active)
        }
        Action::SetParticleSize(value) => {
            update_setting(SnowOption::ParticleSize(value)).and_then(restart_if_active)
        }
        Action::ClearAccumulatedSnow => crate::control::clear_accumulated_snow(),
        Action::OpenConfig => {
            let path = config::ensure_config_file()?;
            Command::new("xdg-open")
                .arg(&path)
                .spawn()
                .map(|_| ())
                .map_err(|error| format!("cannot open {}: {error}", path.display()))
        }
        Action::Exit => Ok(()),
    }
}

fn update_setting(option: SnowOption) -> Result<Settings, String> {
    config::update_snow_option(option)
}

fn restart_if_active(_settings: Settings) -> Result<(), String> {
    if crate::service_is_active()? {
        crate::systemd_action("restart")?;
    }
    Ok(())
}

fn refresh_tray(handle: &ksni::blocking::Handle<SnowTray>) {
    let settings = match Settings::load() {
        Ok(settings) => Some(settings),
        Err(error) => {
            eprintln!("christmas-snow tray: cannot reload configuration: {error}");
            None
        }
    };
    let service_active = match crate::service_is_active() {
        Ok(active) => Some(active),
        Err(error) => {
            eprintln!("christmas-snow tray: cannot check overlay service: {error}");
            None
        }
    };
    if settings.is_none() && service_active.is_none() {
        return;
    }
    match handle.update(|tray| {
        let mut changed = false;
        if let Some(settings) = settings
            && tray.settings != settings
        {
            tray.settings = settings;
            changed = true;
        }
        if let Some(service_active) = service_active
            && tray.service_active != service_active
        {
            tray.service_active = service_active;
            changed = true;
        }
        changed
    }) {
        None => eprintln!("christmas-snow tray: tray service has stopped"),
        Some(false) => {}
        Some(true) => {}
    }
}

fn snowflake_icon() -> ksni::Icon {
    let mut data = vec![0; 32 * 32 * 4];
    let center = (15.5_f32, 15.5_f32);
    for angle in [0.0_f32, 60.0, 120.0] {
        let angle = angle.to_radians();
        let direction = (angle.cos(), angle.sin());
        draw_icon_line(
            &mut data,
            center,
            (center.0 + direction.0 * 13.0, center.1 + direction.1 * 13.0),
        );
        for distance in [6.0_f32, 10.0] {
            let joint = (
                center.0 + direction.0 * distance,
                center.1 + direction.1 * distance,
            );
            for branch in [-2.4_f32, 2.4] {
                let branch_angle = angle + branch;
                draw_icon_line(
                    &mut data,
                    joint,
                    (
                        joint.0 + branch_angle.cos() * 4.0,
                        joint.1 + branch_angle.sin() * 4.0,
                    ),
                );
            }
        }
    }
    ksni::Icon {
        width: 32,
        height: 32,
        data,
    }
}

fn draw_icon_line(data: &mut [u8], start: (f32, f32), end: (f32, f32)) {
    let steps = ((end.0 - start.0).abs().max((end.1 - start.1).abs()).ceil() as usize).max(1);
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let x = (start.0 + (end.0 - start.0) * t).round() as usize;
        let y = (start.1 + (end.1 - start.1) * t).round() as usize;
        let offset = (y * 32 + x) * 4;
        data[offset..offset + 4].copy_from_slice(&[255, 220, 245, 255]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_selection_chooses_nearest_value() {
        assert_eq!(closest_preset(0.7, &DENSITY_PRESETS), 2);
        assert_eq!(closest_preset(0.6, &DENSITY_PRESETS), 2);
        assert_eq!(closest_preset(0.9, &DENSITY_PRESETS), 3);
    }

    #[test]
    fn snowflake_pixmap_has_argb32_dimensions() {
        let icon = snowflake_icon();
        assert_eq!((icon.width, icon.height), (32, 32));
        assert_eq!(icon.data.len(), 32 * 32 * 4);
        assert!(icon.data.iter().any(|byte| *byte != 0));
    }
}
