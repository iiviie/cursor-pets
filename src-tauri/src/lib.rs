//! cursor-pet: a low-footprint desktop pet that chases your cursor.
//!
//! One process hosts everything: a fullscreen, transparent, click-through
//! overlay window renders the pet, and an on-demand settings window lets you
//! customize it. The Rust side reads the global cursor from the compositor and
//! streams it to the overlay; all animation/physics happens in the (tiny)
//! canvas frontend.

mod cursor;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder,
};

const OVERLAY_TITLE: &str = "cursorpet-overlay";
pub const PETS: &[&str] = &["classic", "dog", "maia", "tora", "vaporwave"];

/// User-facing configuration, persisted as JSON and mirrored to the frontend.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Which sprite sheet to use (one of `PETS`).
    pub pet: String,
    /// Rendered sprite scale (1.0 == 32px tile drawn at 64px base).
    pub scale: f64,
    /// Chase speed in logical pixels per second.
    pub speed: f64,
    /// Distance (px) from cursor at which the pet stops and idles.
    pub follow_gap: f64,
    /// Master follow toggle.
    pub follow: bool,
    /// Whether the pet may fall asleep after idling.
    pub sleep_enabled: bool,
    /// Seconds of idle before the pet gets tired / sleeps.
    pub idle_before_sleep: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pet: "classic".into(),
            scale: 1.0,
            speed: 480.0,
            follow_gap: 42.0,
            follow: true,
            sleep_enabled: true,
            idle_before_sleep: 6.0,
        }
    }
}

/// Shared application state.
pub struct AppState {
    pub config: Mutex<Config>,
    pub config_path: PathBuf,
    pub hypr_sock: Option<PathBuf>,
    pub paused: AtomicBool,
}

impl AppState {
    fn load(config_path: PathBuf) -> Self {
        let config = std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| serde_json::from_str::<Config>(&s).ok())
            .unwrap_or_default();
        Self {
            config: Mutex::new(config),
            config_path,
            hypr_sock: cursor::socket_path(),
            paused: AtomicBool::new(false),
        }
    }

    fn persist(&self) {
        if let Ok(cfg) = self.config.lock() {
            if let Some(parent) = self.config_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(&*cfg) {
                let _ = std::fs::write(&self.config_path, json);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tauri commands (frontend <-> backend bridge)
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_config(state: State<Arc<AppState>>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn list_pets() -> Vec<String> {
    PETS.iter().map(|s| s.to_string()).collect()
}

#[tauri::command]
fn get_cursor(state: State<Arc<AppState>>) -> Option<(i32, i32)> {
    state
        .hypr_sock
        .as_ref()
        .and_then(|s| cursor::cursor_pos(s))
}

/// Logical screen size (matches the coordinate space of `get_cursor`).
#[tauri::command]
fn get_screen(state: State<Arc<AppState>>) -> (i32, i32) {
    state
        .hypr_sock
        .as_ref()
        .map(|s| cursor::primary_monitor_size(s))
        .unwrap_or((1600, 900))
}

#[tauri::command]
fn save_config(app: AppHandle, state: State<Arc<AppState>>, config: Config) {
    {
        let mut guard = state.config.lock().unwrap();
        *guard = config.clone();
    }
    state.persist();
    // Push the new config to every window (overlay reacts live).
    let _ = app.emit("config-changed", &config);
}

// ---------------------------------------------------------------------------
// Window / compositor setup
// ---------------------------------------------------------------------------

/// Register Hyprland window rules for our overlay *before* it maps, so the
/// compositor floats + pins it instead of tiling a fullscreen transparent
/// window into the layout. Rules match on the (specific) window title, so they
/// affect nothing else and harmlessly linger until the next Hyprland restart.
fn apply_overlay_rules(sock: &PathBuf, w: i32, h: i32) {
    // Hyprland 0.55 unified rule syntax: `windowrule = <rule>, match:title ^(re)$`.
    // Rule names use underscores + `on`. These make the transparent overlay
    // behave: floating (never tiles into the layout), pinned across
    // workspaces, un-blurred / un-dimmed / no-shadow (so the desktop shows
    // through cleanly), and critically NEVER focus-stealing — the pet must not
    // interrupt the user's work.
    let m = format!("match:title ^({OVERLAY_TITLE})$");
    let rules = [
        "float on".to_string(),
        "pin on".to_string(),
        "no_blur on".to_string(),
        "no_dim on".to_string(),
        "no_shadow on".to_string(),
        "no_anim on".to_string(),
        "no_focus on".to_string(),
        "no_initial_focus on".to_string(),
        "rounding 0".to_string(),
        format!("size {w} {h}"),
        "move 0 0".to_string(),
    ];
    for rule in rules {
        cursor::dispatch(sock, &format!("keyword windowrule {rule}, {m}"));
    }
}

fn open_settings(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("Cursor Pet — Customize")
        .inner_size(760.0, 620.0)
        .min_inner_size(560.0, 480.0)
        .resizable(true)
        .center()
        .build();
}

fn toggle_pause(app: &AppHandle) {
    let state = app.state::<Arc<AppState>>();
    let now = !state.paused.load(Ordering::Relaxed);
    state.paused.store(now, Ordering::Relaxed);
    if let Some(pet) = app.get_webview_window("pet") {
        if now {
            let _ = pet.hide();
        } else {
            let _ = pet.show();
        }
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let customize = MenuItem::with_id(app, "customize", "Customize…", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Show / Hide Pet", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&customize, &toggle, &sep, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("cursor-pet")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "customize" => open_settings(app),
            "toggle" => toggle_pause(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Left-click the tray icon => open the settings window.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                open_settings(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Poll the global cursor and stream it to the overlay window. Emits only when
/// the position actually changes, so an idle desktop produces zero IPC traffic.
fn spawn_cursor_thread(app: AppHandle) {
    let state = app.state::<Arc<AppState>>().inner().clone();
    thread::spawn(move || {
        let Some(sock) = state.hypr_sock.clone() else {
            eprintln!("[cursor-pet] Hyprland socket not found; cursor tracking disabled");
            return;
        };
        let mut last = (i32::MIN, i32::MIN);
        loop {
            if !state.paused.load(Ordering::Relaxed) {
                if let Some(pos) = cursor::cursor_pos(&sock) {
                    if pos != last {
                        last = pos;
                        let _ = app.emit_to("pet", "cursor", pos);
                    }
                }
            }
            thread::sleep(Duration::from_millis(15));
        }
    });
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run() {
    // Force the X11 (XWayland) GDK backend: it gives us reliable absolute
    // window placement and a compositor-independent path for transparency +
    // click-through, which the desktop-pet overlay depends on.
    //
    // We run on the native Wayland backend: under a fractional-scaled Wayland
    // compositor (Hyprland here at scale 1.2) the Wayland window reports the
    // same logical coordinate space the compositor uses for the cursor, so the
    // overlay maps 1:1 to `cursorpos`. (XWayland instead double-scales the
    // surface, which broke the overlay geometry.)
    if std::env::var_os("GDK_BACKEND").is_none() {
        std::env::set_var("GDK_BACKEND", "wayland");
    }
    // NOTE: we intentionally do NOT set WEBKIT_DISABLE_DMABUF_RENDERER — with
    // the SHM fallback renderer, moving sprite frames leave trails on the
    // transparent surface (damage tracking breaks). The DMABUF renderer paints
    // cleanly.

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_config,
            list_pets,
            get_cursor,
            get_screen,
            save_config
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // Resolve config path and load state.
            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| PathBuf::from("."));
            let state = Arc::new(AppState::load(config_dir.join("config.json")));
            app.manage(state.clone());

            // Figure out the logical screen size and register the overlay
            // window rules *before* the window is created, so Hyprland floats +
            // pins + un-blurs it the moment it maps (rules only affect windows
            // that open after they're set).
            let (sw, sh) = state
                .hypr_sock
                .as_ref()
                .map(|s| cursor::primary_monitor_size(s))
                .unwrap_or((1600, 900));
            if let Some(sock) = state.hypr_sock.as_ref() {
                apply_overlay_rules(sock, sw, sh);
            }

            // Now build the transparent, click-through overlay covering the
            // whole (logical) screen.
            let pet = WebviewWindowBuilder::new(app, "pet", WebviewUrl::App("pet.html".into()))
                .title(OVERLAY_TITLE)
                .inner_size(sw as f64, sh as f64)
                .position(0.0, 0.0)
                .transparent(true)
                .decorations(false)
                .shadow(false)
                .skip_taskbar(true)
                .resizable(false)
                .focused(false)
                .always_on_top(true)
                .build()?;
            // Click-through: the overlay must never intercept input.
            let _ = pet.set_ignore_cursor_events(true);
            // Belt-and-suspenders: nudge size/pos via the compositor too.
            if let Some(sock) = state.hypr_sock.as_ref() {
                let t = format!("title:^({OVERLAY_TITLE})$");
                cursor::dispatch(sock, &format!("dispatch setfloating {t}"));
                cursor::dispatch(sock, &format!("dispatch resizewindowpixel exact {sw} {sh},{t}"));
                cursor::dispatch(sock, &format!("dispatch movewindowpixel exact 0 0,{t}"));
            }

            build_tray(&handle)?;
            spawn_cursor_thread(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the settings window destroys its webview (frees memory);
            // it is recreated on demand. The overlay/app keeps running.
            if window.label() == "settings" {
                if let tauri::WindowEvent::CloseRequested { .. } = event {
                    // default behavior (close/destroy) is what we want
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running cursor-pet");
}
