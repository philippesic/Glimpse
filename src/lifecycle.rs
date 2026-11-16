//! A private per-invocation socket separates request completion from GUI lifetime.
use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
static CHANNEL: OnceLock<Mutex<UnixStream>> = OnceLock::new();
fn blocking_channel(stream: &UnixStream) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))
}
pub fn connect() -> Result<(), String> {
    if let Some(path) = std::env::var_os("GLIMPSE_CHANNEL") {
        let stream = UnixStream::connect(path).map_err(|e| e.to_string())?;
        stream.set_nonblocking(true).map_err(|e| e.to_string())?;
        let _ = CHANNEL.set(Mutex::new(stream));
    }
    Ok(())
}
pub fn notify(status: u8) -> Result<(), String> {
    if let Some(channel) = CHANNEL.get() {
        // Each session sends at most two one-byte messages; no document data enters this socket.
        channel
            .lock()
            .unwrap()
            .write_all(&[status])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn launch(args: &[std::ffi::OsString], wait: bool, picker: bool) -> ! {
    let temporary = tempfile::tempdir().unwrap_or_else(|e| fail(&e.to_string()));
    let socket = temporary.path().join("lifecycle.sock");
    let listener = UnixListener::bind(&socket).unwrap_or_else(|e| fail(&e.to_string()));
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|e| fail(&e.to_string()));
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(args)
        .env("GLIMPSE_GUI", "1")
        .stdin(std::process::Stdio::null());
    if !picker {
        command
            .env("GLIMPSE_CHANNEL", &socket)
            .stdout(std::process::Stdio::null());
        if std::env::var_os("GLIMPSE_TRACE").is_none() {
            command.stderr(std::process::Stdio::null());
        }
    }
    let mut child = command
        .spawn()
        .unwrap_or_else(|e| fail(&format!("Cannot launch: {e}")));
    if picker {
        let status = child.wait().unwrap_or_else(|e| fail(&e.to_string()));
        std::process::exit(status.code().filter(|c| (0..=2).contains(c)).unwrap_or(2));
    }
    let started = Instant::now();
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => fail(&e.to_string()),
        }
        if let Ok(Some(_)) = child.try_wait() {
            fail("GUI failed before its lifecycle connection");
        }
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            fail("GUI startup did not connect within 30 seconds");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    // Darwin can inherit the nonblocking listener flag on accepted sockets.
    // A readiness timeout must wait for the GUI instead of treating EAGAIN as failure.
    blocking_channel(&stream).unwrap_or_else(|e| fail(&e.to_string()));
    let mut message = [0];
    if let Err(e) = stream.read_exact(&mut message) {
        let _ = child.kill();
        let _ = child.wait();
        fail(&format!("GUI did not acknowledge a usable frame: {e}"));
    }
    if message[0] != b'0' {
        fail("GUI could not open the requested files or folder; see its error message");
    }
    if !wait {
        std::process::exit(0);
    }
    stream
        .set_read_timeout(None)
        .unwrap_or_else(|e| fail(&e.to_string()));
    match stream.read_exact(&mut message) {
        Ok(()) => std::process::exit(match message[0] {
            b'0' => 0,
            b'1' => 1,
            _ => 2,
        }),
        Err(_) => {
            let _ = child.wait();
            fail("GUI ended before completing the editing request");
        }
    }
}
fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_waits_for_delayed_gui_and_reports_disconnect() {
        let (mut parent, mut gui) = UnixStream::pair().unwrap();
        parent.set_nonblocking(true).unwrap();
        blocking_channel(&parent).unwrap();
        parent
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            gui.write_all(b"0").unwrap();
        });
        let mut byte = [0];
        parent.read_exact(&mut byte).unwrap();
        assert_eq!(byte, b"0"[..]);
        worker.join().unwrap();
        assert!(parent.read_exact(&mut byte).is_err());
    }
}
