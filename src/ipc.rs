//! A second `inkarchy file.pdf` hands the paths to the window that is already open.
//!
//! If nothing is listening, the caller starts its own window.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
mod sys {
    use super::*;
    use std::os::unix::net::{UnixListener, UnixStream};

    pub struct Inbox {
        listener: UnixListener,
        path: PathBuf,
    }

    impl Inbox {
        pub fn bind_default() -> Option<Self> {
            if cfg!(test) {
                return None;
            }
            bind_at(&socket_path(), true)
        }

        pub fn poll(&self) -> Vec<PathBuf> {
            let mut out = Vec::new();
            loop {
                match self.listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                        let _ = stream.set_nonblocking(false);
                        if let Some(paths) = read_msg(&mut stream) {
                            out.extend(paths);
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
            out
        }
    }

    impl Drop for Inbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    pub fn forward(paths: &[PathBuf]) -> bool {
        forward_to(&socket_path(), paths)
    }

    pub(crate) fn forward_to(socket: &Path, paths: &[PathBuf]) -> bool {
        if paths.is_empty() {
            return false;
        }
        let Ok(mut stream) = UnixStream::connect(socket) else {
            return false;
        };
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        write_msg(&mut stream, paths).is_ok()
    }

    pub(crate) fn bind_at(path: &Path, reclaim_stale: bool) -> Option<Inbox> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match UnixListener::bind(path) {
            Ok(listener) => finish(listener, path),
            Err(_) if reclaim_stale => {
                if UnixStream::connect(path).is_ok() {
                    return None;
                }
                let _ = std::fs::remove_file(path);
                let listener = UnixListener::bind(path).ok()?;
                finish(listener, path)
            }
            Err(_) => None,
        }
    }

    fn finish(listener: UnixListener, path: &Path) -> Option<Inbox> {
        listener.set_nonblocking(true).ok()?;
        Some(Inbox {
            listener,
            path: path.to_path_buf(),
        })
    }

    fn write_msg(stream: &mut UnixStream, paths: &[PathBuf]) -> std::io::Result<()> {
        let payload = serde_json::to_vec(paths)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        if payload.len() > 8 * 1024 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "too many paths",
            ));
        }
        stream.write_all(&(payload.len() as u32).to_le_bytes())?;
        stream.write_all(&payload)?;
        stream.flush()?;
        Ok(())
    }

    fn read_msg(stream: &mut UnixStream) -> Option<Vec<PathBuf>> {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).ok()?;
        let len = u32::from_le_bytes(len_buf) as usize;
        if len > 8 * 1024 * 1024 {
            return None;
        }
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).ok()?;
        serde_json::from_slice(&payload).ok()
    }
}

#[cfg(not(unix))]
mod sys {
    use super::*;

    pub struct Inbox;

    impl Inbox {
        pub fn bind_default() -> Option<Self> {
            None
        }

        pub fn poll(&self) -> Vec<PathBuf> {
            Vec::new()
        }
    }

    pub fn forward(_paths: &[PathBuf]) -> bool {
        false
    }
}

pub use sys::Inbox;

pub fn forward(paths: &[PathBuf]) -> bool {
    sys::forward(paths)
}

fn socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let data = crate::library::data_dir();
    let mut hash = 0xcbf29ce484222325u64;
    for byte in data.to_string_lossy().bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    base.join(format!("inkarchy-{hash:x}.sock"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn forward_reaches_a_listening_window_and_stops_when_it_closes() {
        let dir = std::env::temp_dir().join(format!("inkarchy-ipc-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        let socket = dir.join("inkarchy.sock");
        let inbox = sys::bind_at(&socket, false).expect("bind");
        let files = vec![
            PathBuf::from("/tmp/lecture.pdf"),
            PathBuf::from("/tmp/a.png"),
        ];
        assert!(sys::forward_to(&socket, &files));
        assert_eq!(inbox.poll(), files);
        assert!(inbox.poll().is_empty());
        drop(inbox);
        assert!(!sys::forward_to(&socket, &files));
        assert!(!socket.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forward_without_a_listener_is_false() {
        let socket =
            std::env::temp_dir().join(format!("inkarchy-ipc-missing-{}", uuid::Uuid::new_v4()));
        assert!(!sys::forward_to(
            &socket,
            &[PathBuf::from("/tmp/lecture.pdf")]
        ));
    }
}
