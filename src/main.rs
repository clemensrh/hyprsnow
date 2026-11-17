mod config;
mod control;
mod hyprland;
mod input_region;
mod render;
mod sim;
mod tray;
mod wayland;
mod workspaces;

use std::{
    env,
    process::{Command, ExitCode, Output},
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("christmas-snow: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let settings = config::Settings::load()?;
    let command = env::args().nth(1).unwrap_or_else(|| "start".to_owned());

    match command.as_str() {
        "run" => run_overlay(settings)?,
        "tray" => tray::run(settings)?,
        "start" => systemd_action("start")?,
        "stop" => systemd_action("stop")?,
        "restart" => systemd_action("restart")?,
        "config" => {
            let contents = toml::to_string_pretty(&settings).map_err(|error| error.to_string())?;
            print!("{contents}");
        }
        "status" => print_status()?,
        "toggle" => toggle_service()?,
        "snowman" => return Err("snowman interaction mode is not implemented yet".to_owned()),
        "help" | "--help" | "-h" => {
            println!(
                "Usage: christmas-snow [start|stop|restart|status|toggle|tray|snowman|config]\n\
                 Internal: christmas-snow run"
            );
        }
        unknown => return Err(format!("unknown command '{unknown}'; use --help")),
    }
    Ok(())
}

fn run_overlay(settings: config::Settings) -> Result<(), String> {
    if !settings.general.enabled || !settings.snow.enabled {
        println!("snowfall is disabled in configuration");
        return Ok(());
    }
    wayland::run(settings)
}

pub(crate) fn systemd_action(action: &str) -> Result<(), String> {
    let output = Command::new("systemctl")
        .args(["--user", action, "christmas-snow.service"])
        .output()
        .map_err(|error| format!("cannot run systemctl: {error}"))?;
    report_systemd(output)
}

fn print_status() -> Result<(), String> {
    let output = Command::new("systemctl")
        .args(["--user", "is-active", "christmas-snow.service"])
        .output()
        .map_err(|error| format!("cannot run systemctl: {error}"))?;
    let status = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    println!("christmas-snow.service: {status}");
    Ok(())
}

pub(crate) fn service_is_active() -> Result<bool, String> {
    let output = Command::new("systemctl")
        .args(["--user", "is-active", "christmas-snow.service"])
        .output()
        .map_err(|error| format!("cannot run systemctl: {error}"))?;
    if output.status.success() {
        return Ok(true);
    }
    let status = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    match status.as_str() {
        "inactive" | "failed" | "activating" | "deactivating" => Ok(false),
        "" => {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            Err(if detail.is_empty() {
                format!("systemctl is-active exited with {}", output.status)
            } else {
                detail
            })
        }
        other => Err(format!("unexpected systemd service state '{other}'")),
    }
}

fn toggle_service() -> Result<(), String> {
    if service_is_active()? {
        systemd_action("stop")
    } else {
        systemd_action("start")
    }
}

fn report_systemd(output: Output) -> Result<(), String> {
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if detail.is_empty() {
            format!("systemctl exited with {}", output.status)
        } else {
            detail
        });
    }
    let message = String::from_utf8_lossy(&output.stdout);
    if !message.trim().is_empty() {
        print!("{message}");
    }
    Ok(())
}
