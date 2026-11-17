use std::{
    env, fs,
    io::{Read, Write},
    os::linux::fs::MetadataExt,
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

const CLEAR_COMMAND: &[u8; 6] = b"clear\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlRequest {
    ClearAccumulatedSnow,
}

pub struct ControlSocket {
    path: PathBuf,
}

impl Drop for ControlSocket {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!(
                "christmas-snow: cannot remove control socket {}: {error}",
                self.path.display()
            );
        }
    }
}

pub fn start_listener() -> Result<(ControlSocket, Receiver<ControlRequest>), String> {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let runtime_dir = PathBuf::from(runtime_dir);
    let runtime_owner = fs::metadata(&runtime_dir)
        .map_err(|error| format!("cannot inspect {}: {error}", runtime_dir.display()))?
        .st_uid();
    let path = runtime_dir.join("christmas-snow.sock");
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(&path).is_ok() {
                return Err(format!(
                    "another overlay is already listening at {}",
                    path.display()
                ));
            }
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            if !metadata.file_type().is_socket() || metadata.st_uid() != runtime_owner {
                return Err(format!(
                    "refusing to replace non-owned control socket {}",
                    path.display()
                ));
            }
            fs::remove_file(&path).map_err(|error| {
                format!("cannot remove stale socket {}: {error}", path.display())
            })?;
            UnixListener::bind(&path).map_err(|error| {
                format!("cannot create control socket {}: {error}", path.display())
            })?
        }
        Err(error) => {
            return Err(format!(
                "cannot create control socket {}: {error}",
                path.display()
            ));
        }
    };
    let socket = ControlSocket { path: path.clone() };
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("cannot restrict control socket {}: {error}", path.display()))?;
    listener.set_nonblocking(true).map_err(|error| {
        format!(
            "cannot configure control socket {}: {error}",
            path.display()
        )
    })?;

    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("christmas-snow-control".to_owned())
        .spawn(move || {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => handle_client(stream, &sender),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(error) => {
                        eprintln!("christmas-snow: control socket accept failed: {error}");
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        })
        .map_err(|error| format!("cannot start control socket listener: {error}"))?;

    Ok((socket, receiver))
}

pub fn clear_accumulated_snow() -> Result<(), String> {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let path = PathBuf::from(runtime_dir).join("christmas-snow.sock");
    let mut stream = UnixStream::connect(&path).map_err(|error| {
        format!(
            "cannot connect to overlay control socket {}: {error}",
            path.display()
        )
    })?;
    stream
        .write_all(CLEAR_COMMAND)
        .map_err(|error| format!("cannot send clear request: {error}"))?;
    let mut response = [0; 2];
    stream
        .read_exact(&mut response)
        .map_err(|error| format!("cannot receive clear response: {error}"))?;
    if &response == b"ok" {
        Ok(())
    } else {
        Err("overlay rejected the clear-snow request".to_owned())
    }
}

fn handle_client(mut stream: UnixStream, sender: &mpsc::Sender<ControlRequest>) {
    if let Err(error) = stream.set_read_timeout(Some(Duration::from_secs(2))) {
        eprintln!("christmas-snow: cannot configure control client timeout: {error}");
        return;
    }
    let mut request = [0; CLEAR_COMMAND.len()];
    match stream.read_exact(&mut request) {
        Ok(()) if &request == CLEAR_COMMAND => {
            let response = if sender.send(ControlRequest::ClearAccumulatedSnow).is_ok() {
                b"ok"
            } else {
                b"no"
            };
            if let Err(error) = stream.write_all(response) {
                eprintln!("christmas-snow: cannot reply to control client: {error}");
            }
        }
        Ok(()) => {
            if let Err(error) = stream.write_all(b"no") {
                eprintln!("christmas-snow: cannot reject control client: {error}");
            }
        }
        Err(error) => eprintln!("christmas-snow: invalid control request: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_request_is_acknowledged_and_queued() {
        let (sender, receiver) = mpsc::channel();
        let (mut client, server) = UnixStream::pair().unwrap();
        let handler = thread::spawn(move || handle_client(server, &sender));

        client.write_all(CLEAR_COMMAND).unwrap();
        let mut response = [0; 2];
        client.read_exact(&mut response).unwrap();
        handler.join().unwrap();

        assert_eq!(&response, b"ok");
        assert_eq!(
            receiver.recv().unwrap(),
            ControlRequest::ClearAccumulatedSnow
        );
    }

    #[test]
    fn unknown_control_requests_are_rejected() {
        let (sender, receiver) = mpsc::channel();
        let (mut client, server) = UnixStream::pair().unwrap();
        let handler = thread::spawn(move || handle_client(server, &sender));

        client.write_all(b"other\n").unwrap();
        let mut response = [0; 2];
        client.read_exact(&mut response).unwrap();
        handler.join().unwrap();

        assert_eq!(&response, b"no");
        assert!(receiver.try_recv().is_err());
    }
}
