//! Global cursor position, abstracted across platforms.
//!
//! - **Hyprland (Linux):** read `cursorpos` over the compositor's IPC socket —
//!   one tiny unix round-trip, coordinates in the compositor's logical space,
//!   which matches the overlay window 1:1.
//! - **Everywhere else (Windows, macOS, X11 Linux):** `device_query`, which
//!   reads the OS pointer directly. Coordinates are in the platform's screen
//!   pixel space, matched by `get_screen`.
//!
//! On a non-Hyprland *pure Wayland* session there is no portable global-cursor
//! API; `device_query` needs XWayland. That case is documented as unsupported.

use std::path::PathBuf;

/// Where cursor positions come from on this machine.
pub enum CursorSource {
    /// Hyprland IPC socket path (Linux + Hyprland only).
    #[cfg(unix)]
    Hyprland(PathBuf),
    /// Portable OS pointer query (device_query).
    Native,
}

impl CursorSource {
    /// Pick the best source for the current environment.
    pub fn detect() -> Self {
        #[cfg(unix)]
        {
            if let Some(sock) = socket_path() {
                return CursorSource::Hyprland(sock);
            }
        }
        CursorSource::Native
    }

    /// The Hyprland socket, if this source is Hyprland (used for window rules).
    pub fn hypr_sock(&self) -> Option<&PathBuf> {
        match self {
            #[cfg(unix)]
            CursorSource::Hyprland(p) => Some(p),
            _ => None,
        }
    }

    /// Current global cursor position, if available.
    pub fn cursor(&self) -> Option<(i32, i32)> {
        match self {
            #[cfg(unix)]
            CursorSource::Hyprland(sock) => hypr_cursor_pos(sock),
            CursorSource::Native => native_cursor_pos(),
        }
    }
}

/// One-shot cursor read via device_query. Cheap on Windows/macOS; on Linux it
/// opens an X connection, so the polling thread keeps a persistent handle
/// instead of calling this in a loop (see `spawn_cursor_thread`).
pub fn native_cursor_pos() -> Option<(i32, i32)> {
    use device_query::{DeviceQuery, DeviceState};
    let state = DeviceState::new();
    let m = state.get_mouse();
    Some((m.coords.0, m.coords.1))
}

// ---------------------------------------------------------------------------
// Hyprland IPC (unix only). On non-unix (Windows) these are inert stubs so the
// shared setup code in lib.rs compiles unchanged.
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod hypr {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    pub fn socket_path() -> Option<PathBuf> {
        let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        let runtime =
            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".into());
        let candidates = [
            format!("{runtime}/hypr/{sig}/.socket.sock"),
            format!("/tmp/hypr/{sig}/.socket.sock"),
        ];
        candidates
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
    }

    pub fn request(sock: &PathBuf, cmd: &str) -> Option<String> {
        let mut stream = UnixStream::connect(sock).ok()?;
        stream.write_all(cmd.as_bytes()).ok()?;
        let mut buf = String::new();
        stream.read_to_string(&mut buf).ok()?;
        Some(buf)
    }

    pub fn cursor_pos(sock: &PathBuf) -> Option<(i32, i32)> {
        let reply = request(sock, "cursorpos")?;
        let reply = reply.trim();
        let (x, y) = reply.split_once(',')?;
        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
    }

    pub fn monitor_size(sock: &PathBuf) -> (i32, i32) {
        if let Some(reply) = request(sock, "j/monitors") {
            let w = extract_number(&reply, "\"width\":");
            let h = extract_number(&reply, "\"height\":");
            let scale =
                extract_number(&reply, "\"scale\":").filter(|s| *s > 0.0).unwrap_or(1.0);
            if let (Some(w), Some(h)) = (w, h) {
                return ((w / scale).round() as i32, (h / scale).round() as i32);
            }
        }
        (1920, 1080)
    }

    fn extract_number(haystack: &str, key: &str) -> Option<f64> {
        let idx = haystack.find(key)? + key.len();
        let rest = haystack[idx..].trim_start();
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .unwrap_or(rest.len());
        rest[..end].parse().ok()
    }
}

#[cfg(unix)]
pub fn socket_path() -> Option<PathBuf> {
    hypr::socket_path()
}
#[cfg(unix)]
pub fn dispatch(sock: &PathBuf, cmd: &str) {
    let _ = hypr::request(sock, cmd);
}
#[cfg(unix)]
pub fn hypr_cursor_pos(sock: &PathBuf) -> Option<(i32, i32)> {
    hypr::cursor_pos(sock)
}
#[cfg(unix)]
pub fn hypr_monitor_size(sock: &PathBuf) -> (i32, i32) {
    hypr::monitor_size(sock)
}

// Non-unix (Windows) stubs — never reached at runtime (hypr_sock() is None),
// but keep the shared setup code compiling.
#[cfg(not(unix))]
pub fn socket_path() -> Option<PathBuf> {
    None
}
#[cfg(not(unix))]
pub fn dispatch(_sock: &PathBuf, _cmd: &str) {}
#[cfg(not(unix))]
pub fn hypr_monitor_size(_sock: &PathBuf) -> (i32, i32) {
    (1920, 1080)
}
