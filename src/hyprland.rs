use crate::config::WindowSettings;
use serde_json::Value;
use std::{
    env,
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq)]
pub struct WindowGeometry {
    pub address: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorSnapshot {
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub workspace: i64,
    pub fullscreen: bool,
    pub rounding: f32,
    pub windows: Vec<WindowGeometry>,
}

pub struct HyprTracker {
    pub updates: Receiver<Result<Vec<MonitorSnapshot>, String>>,
}

impl HyprTracker {
    pub fn start(window_settings: WindowSettings) -> Result<Self, String> {
        let paths = socket_paths()?;
        let initial = query(&paths.command, &window_settings)?;
        let (sender, updates) = mpsc::channel();
        let _ = sender.send(Ok(initial));
        thread::Builder::new()
            .name("christmas-snow-hyprland-ipc".to_owned())
            .spawn(move || watch_events(paths.events, paths.command, window_settings, sender))
            .map_err(|error| format!("cannot start Hyprland IPC watcher: {error}"))?;
        Ok(Self { updates })
    }
}

struct SocketPaths {
    command: PathBuf,
    events: PathBuf,
}

fn socket_paths() -> Result<SocketPaths, String> {
    let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let signature = env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
        .ok_or("HYPRLAND_INSTANCE_SIGNATURE is not set")?;
    let directory = PathBuf::from(runtime).join("hypr").join(signature);
    Ok(SocketPaths {
        command: directory.join(".socket.sock"),
        events: directory.join(".socket2.sock"),
    })
}

fn query(path: &PathBuf, settings: &WindowSettings) -> Result<Vec<MonitorSnapshot>, String> {
    let monitors: Value = serde_json::from_slice(&request(path, "j/monitors")?)
        .map_err(|error| format!("invalid Hyprland monitor JSON: {error}"))?;
    let clients: Value = serde_json::from_slice(&request(path, "j/clients")?)
        .map_err(|error| format!("invalid Hyprland client JSON: {error}"))?;
    let rounding: Value =
        serde_json::from_slice(&request(path, "j/getoption decoration:rounding")?)
            .map_err(|error| format!("invalid Hyprland rounding JSON: {error}"))?;
    let rounding = rounding
        .get("int")
        .and_then(Value::as_i64)
        .ok_or("Hyprland rounding option is missing its integer value")?
        .max(0) as f32;
    parse_snapshots(&monitors, &clients, settings, rounding)
}

fn parse_snapshots(
    monitors: &Value,
    clients: &Value,
    settings: &WindowSettings,
    rounding: f32,
) -> Result<Vec<MonitorSnapshot>, String> {
    let monitor_list = monitors
        .as_array()
        .ok_or("Hyprland returned a non-array monitor list")?;
    let client_list = clients
        .as_array()
        .ok_or("Hyprland returned a non-array client list")?;

    monitor_list
        .iter()
        .map(|monitor| {
            let id = number(monitor, "id")? as i32;
            let name = string(monitor, "name")?.to_owned();
            let x = number(monitor, "x")?;
            let y = number(monitor, "y")?;
            let active_workspace = monitor
                .get("activeWorkspace")
                .and_then(|workspace| workspace.get("id"))
                .and_then(Value::as_i64)
                .ok_or_else(|| format!("monitor {name} has no active workspace id"))?;
            let special_workspace = monitor
                .get("specialWorkspace")
                .and_then(|workspace| workspace.get("id"))
                .and_then(Value::as_i64)
                .filter(|id| *id != 0);
            let windows = client_list
                .iter()
                .filter_map(|client| {
                    window_geometry(client, id, active_workspace, special_workspace, settings)
                })
                .collect::<Vec<_>>();
            let fullscreen = client_list.iter().any(|client| {
                visible_client(client, id, active_workspace, special_workspace)
                    && client
                        .get("fullscreen")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                        != 0
            });
            Ok(MonitorSnapshot {
                name,
                x,
                y,
                workspace: special_workspace.unwrap_or(active_workspace),
                fullscreen,
                rounding,
                windows,
            })
        })
        .collect()
}

fn window_geometry(
    client: &Value,
    monitor_id: i32,
    active_workspace: i64,
    special_workspace: Option<i64>,
    settings: &WindowSettings,
) -> Option<WindowGeometry> {
    if !visible_client(client, monitor_id, active_workspace, special_workspace) {
        return None;
    }
    let floating = client.get("floating")?.as_bool()?;
    if (floating && !settings.track_floating) || (!floating && !settings.track_tiled) {
        return None;
    }
    let address = string(client, "address").ok()?.to_owned();
    let at = client.get("at")?.as_array()?;
    let size = client.get("size")?.as_array()?;
    let x = at.first()?.as_f64()? as f32;
    let y = at.get(1)?.as_f64()? as f32;
    let width = size.first()?.as_f64()? as f32;
    let height = size.get(1)?.as_f64()? as f32;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(WindowGeometry {
        address,
        x,
        y,
        width,
        height,
    })
}

fn visible_client(
    client: &Value,
    monitor_id: i32,
    active_workspace: i64,
    special_workspace: Option<i64>,
) -> bool {
    let workspace = client
        .get("workspace")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_i64);
    client.get("mapped").and_then(Value::as_bool) == Some(true)
        && client.get("hidden").and_then(Value::as_bool) == Some(false)
        && client.get("monitor").and_then(Value::as_i64) == Some(i64::from(monitor_id))
        && (client.get("pinned").and_then(Value::as_bool) == Some(true)
            || workspace == Some(active_workspace)
            || (special_workspace.is_some() && workspace == special_workspace))
}

fn request(path: &PathBuf, command: &str) -> Result<Vec<u8>, String> {
    let mut socket = UnixStream::connect(path).map_err(|error| {
        format!(
            "cannot connect to Hyprland IPC at {}: {error}",
            path.display()
        )
    })?;
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| format!("cannot configure Hyprland IPC timeout: {error}"))?;
    socket
        .write_all(command.as_bytes())
        .map_err(|error| format!("cannot send Hyprland IPC request: {error}"))?;
    socket
        .shutdown(std::net::Shutdown::Write)
        .map_err(|error| format!("cannot finish Hyprland IPC request: {error}"))?;
    let mut response = Vec::new();
    socket
        .read_to_end(&mut response)
        .map_err(|error| format!("cannot read Hyprland IPC response: {error}"))?;
    Ok(response)
}

fn watch_events(
    events_path: PathBuf,
    command_path: PathBuf,
    settings: WindowSettings,
    sender: Sender<Result<Vec<MonitorSnapshot>, String>>,
) {
    let mut previous = None;
    let mut last_error = None;
    loop {
        let mut stream = match UnixStream::connect(&events_path) {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!(
                    "christmas-snow: cannot connect to Hyprland event socket {}: {error}",
                    events_path.display()
                );
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        if let Err(error) = stream.set_nonblocking(true) {
            if sender
                .send(Err(format!(
                    "cannot configure Hyprland event socket: {error}"
                )))
                .is_err()
            {
                return;
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        }
        let mut pending = Vec::with_capacity(4096);
        let mut next_query = Instant::now();
        loop {
            let mut buffer = [0; 4096];
            let mut changed = false;
            let mut disconnected = false;
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => {
                        disconnected = true;
                        break;
                    }
                    Ok(count) => changed |= has_relevant_event(&mut pending, &buffer[..count]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => {
                        eprintln!("christmas-snow: Hyprland event socket read failed: {error}");
                        disconnected = true;
                        break;
                    }
                }
            }
            if disconnected {
                break;
            }
            if changed || Instant::now() >= next_query {
                match query(&command_path, &settings) {
                    Ok(snapshot) => {
                        if previous.as_ref() != Some(&snapshot) {
                            previous = Some(snapshot.clone());
                            if sender.send(Ok(snapshot)).is_err() {
                                return;
                            }
                        }
                        last_error = None;
                    }
                    Err(error) if last_error.as_ref() != Some(&error) => {
                        last_error = Some(error.clone());
                        if sender.send(Err(error)).is_err() {
                            return;
                        }
                    }
                    Err(_) => {}
                }
                next_query = Instant::now() + Duration::from_millis(100);
            }
            thread::sleep(Duration::from_millis(20));
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn has_relevant_event(pending: &mut Vec<u8>, bytes: &[u8]) -> bool {
    pending.extend_from_slice(bytes);
    let mut consumed = 0;
    let mut changed = false;
    while let Some(end) = pending[consumed..].iter().position(|byte| *byte == b'\n') {
        let line_end = consumed + end;
        let name_end = pending[consumed..line_end]
            .iter()
            .position(|byte| *byte == b'>')
            .map_or(line_end, |separator| consumed + separator);
        changed |= relevant_event(&pending[consumed..name_end]);
        consumed = line_end + 1;
    }
    if consumed > 0 {
        pending.drain(..consumed);
    }
    changed
}

fn relevant_event(name: &[u8]) -> bool {
    matches!(
        name,
        b"openwindow"
            | b"closewindow"
            | b"movewindow"
            | b"movewindowv2"
            | b"changefloatingmode"
            | b"workspace"
            | b"workspacev2"
            | b"activespecial"
            | b"activespecialv2"
            | b"focusedmon"
            | b"fullscreen"
            | b"monitoradded"
            | b"monitorremoved"
            | b"monitorconfig"
    )
}
fn string<'a>(object: &'a Value, key: &str) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Hyprland response is missing string field '{key}'"))
}

fn number(object: &Value, key: &str) -> Result<f32, String> {
    object
        .get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .ok_or_else(|| format!("Hyprland response is missing numeric field '{key}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        os::unix::net::UnixListener,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    fn client(address: &str, workspace: i64, floating: bool) -> Value {
        json!({
            "address": address, "mapped": true, "hidden": false, "monitor": 0,
            "workspace": {"id": workspace}, "floating": floating, "pinned": false,
            "at": [5, 10], "size": [40, 30], "fullscreen": 0
        })
    }

    #[test]
    fn snapshot_tracks_regular_special_and_pinned_windows() {
        let monitors = json!([{
            "id": 0, "name": "output", "x": 0, "y": 0,
            "activeWorkspace": {"id": 1}, "specialWorkspace": {"id": -99}
        }]);
        let mut pinned = client("pinned", 3, true);
        pinned["pinned"] = json!(true);
        let clients = json!([
            client("regular", 1, false),
            client("special", -99, true),
            client("inactive", 2, false),
            pinned
        ]);
        let snapshots =
            parse_snapshots(&monitors, &clients, &WindowSettings::default(), 10.0).unwrap();
        assert_eq!(snapshots[0].workspace, -99);
        let addresses: Vec<_> = snapshots[0]
            .windows
            .iter()
            .map(|w| w.address.as_str())
            .collect();
        assert_eq!(addresses, ["regular", "special", "pinned"]);
        assert_eq!(snapshots[0].windows[0].height, 30.0);
    }

    #[test]
    fn fullscreen_detection_does_not_depend_on_window_tracking_filters() {
        let monitors = json!([{
            "id": 0, "name": "output", "x": 0, "y": 0,
            "activeWorkspace": {"id": 1}, "specialWorkspace": {"id": 0}
        }]);
        let mut fullscreen = client("fullscreen", 1, true);
        fullscreen["fullscreen"] = json!(2);
        let settings = WindowSettings {
            track_floating: false,
            ..WindowSettings::default()
        };
        let snapshots = parse_snapshots(&monitors, &json!([fullscreen]), &settings, 10.0).unwrap();
        assert_eq!(snapshots[0].workspace, 1);
        assert!(snapshots[0].fullscreen);
        assert!(snapshots[0].windows.is_empty());
    }

    #[test]
    fn event_names_include_workspace_and_window_changes() {
        for event in [
            b"openwindow".as_slice(),
            b"closewindow",
            b"movewindowv2",
            b"workspacev2",
            b"activespecial",
        ] {
            assert!(relevant_event(event));
        }
        assert!(!relevant_event(b"mouse"));
    }

    #[test]
    fn event_buffer_handles_fragmented_and_batched_lines() {
        let mut pending = Vec::new();
        assert!(!has_relevant_event(&mut pending, b"openwin"));
        assert!(has_relevant_event(
            &mut pending,
            b"dow>>address\nmouse>>move\nclosewindow>>address\n"
        ));
        assert!(pending.is_empty());
    }

    #[test]
    fn watcher_detects_geometry_without_events_and_workspace_events() {
        let directory = env::temp_dir().join(format!(
            "christmas-snow-ipc-test-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let command_path = directory.join("command");
        let events_path = directory.join("events");
        let command_listener = UnixListener::bind(&command_path).unwrap();
        command_listener.set_nonblocking(true).unwrap();
        let event_listener = UnixListener::bind(&events_path).unwrap();
        let state = Arc::new(Mutex::new((
            json!([{"id": 0, "name": "output", "x": 0, "y": 0,
                "activeWorkspace": {"id": 1}, "specialWorkspace": {"id": 0}}]),
            json!([]),
        )));
        let running = Arc::new(AtomicBool::new(true));
        let server_state = Arc::clone(&state);
        let server_running = Arc::clone(&running);
        let server = thread::spawn(move || {
            while server_running.load(Ordering::Relaxed) {
                match command_listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut command = String::new();
                        stream.read_to_string(&mut command).unwrap();
                        let state = server_state.lock().unwrap();
                        let response = match command.as_str() {
                            "j/monitors" => state.0.clone(),
                            "j/clients" => state.1.clone(),
                            "j/getoption decoration:rounding" => json!({"int": 10}),
                            _ => panic!("unexpected IPC request {command}"),
                        };
                        stream.write_all(response.to_string().as_bytes()).unwrap();
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("test IPC server failed: {error}"),
                }
            }
        });
        let (sender, receiver) = mpsc::channel();
        let watcher = thread::spawn(move || {
            watch_events(events_path, command_path, WindowSettings::default(), sender)
        });
        let (mut events, _) = event_listener.accept().unwrap();
        let timeout = Duration::from_secs(3);
        let initial = receiver.recv_timeout(timeout).unwrap().unwrap();
        assert_eq!(initial[0].workspace, 1);
        assert!(initial[0].windows.is_empty());

        state.lock().unwrap().1 = json!([client("new", 1, false)]);
        let created = receiver.recv_timeout(timeout).unwrap().unwrap();
        assert_eq!(created[0].windows[0].width, 40.0);
        state.lock().unwrap().1[0]["size"] = json!([65, 30]);
        let resized = receiver.recv_timeout(timeout).unwrap().unwrap();
        assert_eq!(resized[0].windows[0].width, 65.0);
        state.lock().unwrap().1[0]["at"] = json!([22, 16]);
        let moved = receiver.recv_timeout(timeout).unwrap().unwrap();
        assert_eq!((moved[0].windows[0].x, moved[0].windows[0].y), (22.0, 16.0));

        state.lock().unwrap().1 = json!([]);
        events.write_all(b"closewindow>>new\n").unwrap();
        let closed = receiver.recv_timeout(timeout).unwrap().unwrap();
        assert!(closed[0].windows.is_empty());

        {
            let mut state = state.lock().unwrap();
            state.0[0]["activeWorkspace"]["id"] = json!(2);
            state.1 = json!([]);
        }
        events.write_all(b"workspacev2>>2,2\n").unwrap();
        let switched = receiver.recv_timeout(timeout).unwrap().unwrap();
        assert_eq!(switched[0].workspace, 2);
        assert!(switched[0].windows.is_empty());

        state.lock().unwrap().0[0]["activeWorkspace"]["id"] = json!(1);
        events.write_all(b"workspacev2>>1,1\n").unwrap();
        let returned = receiver.recv_timeout(timeout).unwrap().unwrap();
        assert_eq!(returned[0].workspace, 1);

        drop(receiver);
        state.lock().unwrap().0[0]["activeWorkspace"]["id"] = json!(3);
        events.write_all(b"workspacev2>>3,3\n").unwrap();
        watcher.join().unwrap();
        running.store(false, Ordering::Relaxed);
        server.join().unwrap();
        std::fs::remove_file(directory.join("command")).unwrap();
        std::fs::remove_file(directory.join("events")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
