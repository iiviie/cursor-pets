//! Global cursor position + Hyprland IPC helpers.
//!
//! On Hyprland the compositor exposes the global cursor position over its IPC
//! socket (`cursorpos`), returning coordinates in the same logical space the
//! overlay window lives in. We talk to that socket directly (one tiny unix
//! socket round-trip, ~tens of microseconds) instead of spawning `hyprctl`,
//! so polling at 60 Hz costs almost nothing.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// Locate the Hyprland command socket for the current instance.
pub fn socket_path() -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".into());
    let candidates = [
        format!("{runtime}/hypr/{sig}/.socket.sock"),
        format!("/tmp/hypr/{sig}/.socket.sock"),
    ];
    candidates
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

/// Send a single command to the Hyprland IPC socket and return the reply.
pub fn request(sock: &PathBuf, cmd: &str) -> Option<String> {
    let mut stream = UnixStream::connect(sock).ok()?;
    stream.write_all(cmd.as_bytes()).ok()?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok()?;
    Some(buf)
}

/// Fire-and-mostly-forget: send a command, ignore the reply body.
pub fn dispatch(sock: &PathBuf, cmd: &str) {
    let _ = request(sock, cmd);
}

/// Current global cursor position in logical pixels, or None if unavailable.
pub fn cursor_pos(sock: &PathBuf) -> Option<(i32, i32)> {
    let reply = request(sock, "cursorpos")?;
    let reply = reply.trim();
    let (x, y) = reply.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Logical size of the primary monitor, falling back to 1920x1080.
pub fn primary_monitor_size(sock: &PathBuf) -> (i32, i32) {
    if let Some(reply) = request(sock, "j/monitors") {
        // Minimal hand-parse to avoid pulling coordinates through serde_json for
        // a one-shot startup query. Look for the first "width"/"height"/"scale".
        let w = extract_number(&reply, "\"width\":");
        let h = extract_number(&reply, "\"height\":");
        let scale = extract_number(&reply, "\"scale\":").filter(|s| *s > 0.0).unwrap_or(1.0);
        if let (Some(w), Some(h)) = (w, h) {
            return ((w / scale).round() as i32, (h / scale).round() as i32);
        }
    }
    (1920, 1080)
}

fn extract_number(haystack: &str, key: &str) -> Option<f64> {
    let idx = haystack.find(key)? + key.len();
    let rest = &haystack[idx..];
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}
