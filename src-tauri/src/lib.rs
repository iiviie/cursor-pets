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
    /// Which sprite sheet to use (a built-in in `PETS` or a custom pet id).
    pub pet: String,
    /// Rendered sprite scale (1.0 == 32px tile drawn at 64px base).
    pub scale: f64,
    /// Chase speed in logical pixels per second.
    pub speed: f64,
    /// Distance (px) from cursor at which the pet stops and idles.
    pub follow_gap: f64,
    /// Cursor-follow smoothing time in seconds. Higher = laggier/calmer; the
    /// pet chases a smoothed target so slow mouse wiggles don't make it twitch.
    pub reaction: f64,
    /// Sprite opacity, 0..1 (1 = fully opaque).
    pub opacity: f64,
    /// Master follow toggle.
    pub follow: bool,
    /// Whether the pet may fall asleep after idling.
    pub sleep_enabled: bool,
    /// Seconds of idle before the pet gets tired / sleeps.
    pub idle_before_sleep: f64,
    /// Whether the pet does occasional idle fidgets (grooming).
    pub fidget_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pet: "classic".into(),
            scale: 0.7,
            speed: 220.0,
            follow_gap: 70.0,
            reaction: 0.16,
            opacity: 1.0,
            follow: true,
            sleep_enabled: true,
            idle_before_sleep: 6.0,
            fidget_enabled: true,
        }
    }
}

impl Config {
    /// Clamp values into sane ranges so a hand-edited or corrupt config.json
    /// can't make the pet teleport, vanish, or spin. Non-finite numbers
    /// (NaN/Infinity) fall back to the default.
    fn normalize(&mut self) {
        let d = Config::default();
        let fix = |v: f64, def: f64, lo: f64, hi: f64| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                def
            }
        };
        self.scale = fix(self.scale, d.scale, 0.2, 4.0);
        self.opacity = fix(self.opacity, d.opacity, 0.05, 1.0);
        self.speed = fix(self.speed, d.speed, 20.0, 2000.0);
        self.follow_gap = fix(self.follow_gap, d.follow_gap, 0.0, 600.0);
        self.reaction = fix(self.reaction, d.reaction, 0.0, 2.0);
        self.idle_before_sleep = fix(self.idle_before_sleep, d.idle_before_sleep, 0.5, 600.0);
        if self.pet.trim().is_empty() {
            self.pet = d.pet;
        }
    }
}

/// Metadata for a pet the frontend can choose.
#[derive(Serialize, Clone, Debug)]
pub struct PetInfo {
    /// Stable id used everywhere (built-in name, or custom file stem).
    pub id: String,
    /// True for the bundled pets, false for user-imported ones.
    pub builtin: bool,
}

/// Shared application state.
pub struct AppState {
    pub config: Mutex<Config>,
    pub config_path: PathBuf,
    pub pets_dir: PathBuf,
    pub cursor: cursor::CursorSource,
    pub paused: AtomicBool,
}

impl AppState {
    fn load(config_dir: PathBuf) -> Self {
        let config_path = config_dir.join("config.json");
        let pets_dir = config_dir.join("pets");
        let mut config = std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| serde_json::from_str::<Config>(&s).ok())
            .unwrap_or_default();
        config.normalize();
        Self {
            config: Mutex::new(config),
            config_path,
            pets_dir,
            cursor: cursor::CursorSource::detect(),
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
fn list_pets(state: State<Arc<AppState>>) -> Vec<PetInfo> {
    let mut pets: Vec<PetInfo> = PETS
        .iter()
        .map(|s| PetInfo {
            id: s.to_string(),
            builtin: true,
        })
        .collect();
    if let Ok(entries) = std::fs::read_dir(&state.pets_dir) {
        let mut custom: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("png") {
                    p.file_stem().and_then(|s| s.to_str()).map(String::from)
                } else {
                    None
                }
            })
            .collect();
        custom.sort();
        pets.extend(custom.into_iter().map(|id| PetInfo { id, builtin: false }));
    }
    pets
}

/// Return an `<img>`-loadable source for a pet: the bundled asset path for
/// built-ins, or a base64 data URL for a custom (out-of-bundle) sheet.
#[tauri::command]
fn pet_src(state: State<Arc<AppState>>, id: String) -> Result<String, String> {
    if PETS.contains(&id.as_str()) {
        return Ok(format!("sprites/oneko-{id}.png"));
    }
    let path = custom_pet_path(&state.pets_dir, &id).ok_or("invalid pet id")?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:image/png;base64,{b64}"))
}

/// Return the animation manifest for a custom pet (state->frames overrides),
/// or null if it has none / is a built-in.
#[tauri::command]
fn pet_manifest(state: State<Arc<AppState>>, id: String) -> Option<serde_json::Value> {
    if PETS.contains(&id.as_str()) {
        return None;
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return None;
    }
    let path = state.pets_dir.join(format!("{id}.json"));
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Open a file picker, validate the chosen PNG is an oneko-style sprite sheet,
/// copy it into the pets dir, and return its new entry.
#[tauri::command]
async fn import_pet(app: AppHandle) -> Result<Option<PetInfo>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .add_filter("Sprite sheet (PNG)", &["png"])
        .blocking_pick_file();
    let Some(picked) = picked else {
        return Ok(None); // user cancelled
    };
    let src = picked.into_path().map_err(|e| e.to_string())?;

    let bytes = std::fs::read(&src).map_err(|e| e.to_string())?;
    let (w, h) = png_dimensions(&bytes)
        .ok_or("That file isn't a valid PNG.")?;
    // Sheets are an 8x4 grid of 32px tiles, so both dims must be multiples of
    // 32 and at least 256x128.
    if w % 32 != 0 || h % 32 != 0 || w < 256 || h < 128 {
        return Err(format!(
            "Sprite sheet must be an 8x4 grid of 32px tiles (≥256x128, sizes multiple of 32). Got {w}x{h}."
        ));
    }
    // Guard against absurd sheets (they'd become huge base64 data URLs).
    if w > 4096 || h > 4096 {
        return Err(format!("Sprite sheet is too large ({w}x{h}); max 4096x4096."));
    }

    let state = app.state::<Arc<AppState>>();
    std::fs::create_dir_all(&state.pets_dir).map_err(|e| e.to_string())?;
    let id = unique_pet_id(&state.pets_dir, &src);
    let dest = state.pets_dir.join(format!("{id}.png"));
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
    // If a sibling manifest (<sheet>.json) sits next to the sheet, bring it
    // along so the pet keeps its custom state/frame mapping.
    if let Ok(manifest) = std::fs::read(src.with_extension("json")) {
        if serde_json::from_slice::<serde_json::Value>(&manifest).is_ok() {
            let _ = std::fs::write(state.pets_dir.join(format!("{id}.json")), &manifest);
        }
    }
    Ok(Some(PetInfo { id, builtin: false }))
}

#[tauri::command]
fn delete_pet(app: AppHandle, state: State<Arc<AppState>>, id: String) -> Result<(), String> {
    if PETS.contains(&id.as_str()) {
        return Err("Built-in pets can't be deleted.".into());
    }
    let path = custom_pet_path(&state.pets_dir, &id).ok_or("invalid pet id")?;
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(state.pets_dir.join(format!("{id}.json")));
    // If the deleted pet was selected, fall back to the default and notify.
    let mut fell_back = None;
    {
        let mut cfg = state.config.lock().unwrap();
        if cfg.pet == id {
            cfg.pet = "classic".into();
            fell_back = Some(cfg.clone());
        }
    }
    if let Some(cfg) = fell_back {
        state.persist();
        let _ = app.emit("config-changed", &cfg);
    }
    Ok(())
}

/// Resolve a custom pet id to its file, rejecting path-traversal ids.
fn custom_pet_path(pets_dir: &PathBuf, id: &str) -> Option<PathBuf> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return None;
    }
    Some(pets_dir.join(format!("{id}.png")))
}

/// Sanitize the source file name into a unique, filesystem-safe pet id.
fn unique_pet_id(pets_dir: &PathBuf, src: &PathBuf) -> String {
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("pet")
        .to_lowercase();
    let base: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let base = base.trim_matches('-');
    let base = if base.is_empty() { "pet" } else { base };
    // Avoid colliding with built-ins or existing files.
    let mut id = base.to_string();
    let mut n = 2;
    while PETS.contains(&id.as_str()) || pets_dir.join(format!("{id}.png")).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Parse width/height from a PNG's IHDR without pulling in an image decoder.
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIG {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((w, h))
}

/// Whether the app is registered to launch on login (OS-level, cross-platform).
#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Enable/disable launch-on-login. On Linux this writes an autostart .desktop,
/// on Windows a Run registry key, on macOS a Launch Agent.
#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    if enabled {
        mgr.enable().map_err(|e| e.to_string())
    } else {
        mgr.disable().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn get_cursor(state: State<Arc<AppState>>) -> Option<(i32, i32)> {
    state.cursor.cursor()
}

/// Extent of the coordinate space `get_cursor` reports in, so the overlay can
/// map cursor positions into its own CSS pixels. On Hyprland that's the logical
/// monitor size; on other platforms it's the native (physical) monitor size,
/// matching what device_query returns.
#[tauri::command]
fn get_screen(app: AppHandle, state: State<Arc<AppState>>) -> (i32, i32) {
    match state.cursor.hypr_sock() {
        Some(sock) => cursor::hypr_monitor_size(sock),
        None => native_physical_size(&app),
    }
}

/// Physical size of the primary monitor via Tauri (used off Hyprland).
fn native_physical_size(app: &AppHandle) -> (i32, i32) {
    app.primary_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let s = m.size();
            (s.width as i32, s.height as i32)
        })
        .unwrap_or((1920, 1080))
}

/// Logical size to give the overlay window so it covers the primary monitor.
fn overlay_logical_size(app: &AppHandle, state: &AppState) -> (i32, i32) {
    match state.cursor.hypr_sock() {
        Some(sock) => cursor::hypr_monitor_size(sock),
        None => app
            .primary_monitor()
            .ok()
            .flatten()
            .map(|m| {
                let s = m.size();
                let sf = m.scale_factor().max(0.1);
                (
                    ((s.width as f64) / sf).round() as i32,
                    ((s.height as f64) / sf).round() as i32,
                )
            })
            .unwrap_or((1600, 900)),
    }
}

#[tauri::command]
fn save_config(app: AppHandle, state: State<Arc<AppState>>, mut config: Config) {
    config.normalize();
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
        // Force full opacity so the compositor's inactive_opacity (our overlay
        // is never focused) doesn't make the sprite translucent. Per-pet
        // opacity is applied in the canvas instead.
        "opacity 1.0 override 1.0 override 1.0".to_string(),
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
    // In test mode the settings page auto-drives a config change so we can
    // verify live-apply end-to-end without synthetic clicks.
    let url = if std::env::var_os("CURSORPET_AUTOTEST").is_some() {
        "settings.html?autotest=1"
    } else {
        "settings.html"
    };
    let _ = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App(url.into()))
        .title("Cursor Pet — Customize")
        .inner_size(840.0, 600.0)
        .min_inner_size(620.0, 520.0)
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

/// Check GitHub Releases for a newer version and install it.
/// - `interactive` (tray "Check for updates…"): report the outcome via a dialog
///   and offer to restart. Otherwise (startup) run silently; the update applies
///   on the next launch.
#[cfg(desktop)]
async fn perform_update(app: AppHandle, interactive: bool) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    use tauri_plugin_updater::UpdaterExt;

    let notify = |msg: String, kind: MessageDialogKind| {
        app.dialog()
            .message(msg)
            .title("cursor-pet")
            .kind(kind)
            .show(|_| {});
    };

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            if interactive {
                notify(format!("Updater unavailable: {e}"), MessageDialogKind::Error);
            }
            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            let version = update.version.clone();
            match update.download_and_install(|_, _| {}, || {}).await {
                Ok(_) => {
                    if interactive {
                        let app2 = app.clone();
                        app.dialog()
                            .message(format!(
                                "Updated to v{version}. Restart cursor-pet now to apply?"
                            ))
                            .title("cursor-pet")
                            .buttons(MessageDialogButtons::OkCancelCustom(
                                "Restart".into(),
                                "Later".into(),
                            ))
                            .show(move |restart| {
                                if restart {
                                    app2.restart();
                                }
                            });
                    }
                    // Silent path: installed; takes effect on next launch.
                }
                Err(e) => {
                    if interactive {
                        notify(format!("Update failed: {e}"), MessageDialogKind::Error);
                    }
                }
            }
        }
        Ok(None) => {
            if interactive {
                notify(
                    "You're on the latest version.".into(),
                    MessageDialogKind::Info,
                );
            }
        }
        Err(e) => {
            if interactive {
                notify(
                    format!("Couldn't check for updates: {e}"),
                    MessageDialogKind::Warning,
                );
            }
        }
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let customize = MenuItem::with_id(app, "customize", "Customize…", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Show / Hide Pet", true, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "Check for updates…", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&customize, &toggle, &update, &sep, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("cursor-pet")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "customize" => open_settings(app),
            "toggle" => toggle_pause(app),
            "update" => {
                #[cfg(desktop)]
                {
                    let app = app.clone();
                    tauri::async_runtime::spawn(perform_update(app, true));
                }
            }
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
        let mut last = (i32::MIN, i32::MIN);
        // Persistent device_query handle for the Native path (avoids
        // reconnecting to the OS/X each poll). Not used on Hyprland.
        let native = device_query::DeviceState::new();
        loop {
            if !state.paused.load(Ordering::Relaxed) {
                let pos = match &state.cursor {
                    #[cfg(unix)]
                    cursor::CursorSource::Hyprland(sock) => cursor::hypr_cursor_pos(sock),
                    cursor::CursorSource::Native => {
                        use device_query::DeviceQuery;
                        let m = native.get_mouse();
                        Some((m.coords.0, m.coords.1))
                    }
                };
                if let Some(pos) = pos {
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
    // NOTE: we intentionally do NOT set WEBKIT_DISABLE_DMABUF_RENDERER — with
    // the SHM fallback renderer, moving sprite frames leave trails on the
    // transparent surface (damage tracking breaks). The DMABUF renderer paints
    // cleanly.
    #[cfg(target_os = "linux")]
    if std::env::var_os("GDK_BACKEND").is_none() {
        std::env::set_var("GDK_BACKEND", "wayland");
    }

    let mut builder = tauri::Builder::default()
        // Only one pet, please: a second launch just opens the settings window
        // of the already-running instance instead of spawning another overlay.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            open_settings(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }

    builder
        .invoke_handler(tauri::generate_handler![
            get_config,
            list_pets,
            pet_src,
            pet_manifest,
            import_pet,
            delete_pet,
            get_cursor,
            get_screen,
            get_autostart,
            set_autostart,
            save_config
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // Resolve config path and load state.
            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| PathBuf::from("."));
            let state = Arc::new(AppState::load(config_dir));
            app.manage(state.clone());

            // Figure out the logical screen size and (on Hyprland) register the
            // overlay window rules *before* the window is created, so the
            // compositor floats + pins + un-blurs it the moment it maps (rules
            // only affect windows that open after they're set). On other
            // platforms the native Tauri window flags below are enough.
            let (sw, sh) = overlay_logical_size(&handle, &state);
            if let Some(sock) = state.cursor.hypr_sock() {
                apply_overlay_rules(sock, sw, sh);
                // Float + center the settings window (title starts with
                // "Cursor Pet") so a tiling WM doesn't wedge it into a tile.
                cursor::dispatch(sock, "keyword windowrule float on, match:title ^(Cursor Pet).*$");
                cursor::dispatch(sock, "keyword windowrule center on, match:title ^(Cursor Pet).*$");
                cursor::dispatch(sock, "keyword windowrule no_dim on, match:title ^(Cursor Pet).*$");
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
            if let Some(sock) = state.cursor.hypr_sock() {
                let t = format!("title:^({OVERLAY_TITLE})$");
                cursor::dispatch(sock, &format!("dispatch setfloating {t}"));
                cursor::dispatch(sock, &format!("dispatch resizewindowpixel exact {sw} {sh},{t}"));
                cursor::dispatch(sock, &format!("dispatch movewindowpixel exact 0 0,{t}"));
            }

            build_tray(&handle)?;
            spawn_cursor_thread(handle.clone());

            // Optional: auto-open settings on launch (handy for testing).
            if std::env::var_os("CURSORPET_SETTINGS").is_some() {
                open_settings(&handle);
            }

            // Quietly check for updates on launch; if one is found it installs
            // in the background and applies on the next start (no interruption).
            #[cfg(desktop)]
            if std::env::var_os("CURSORPET_NO_UPDATE_CHECK").is_none() {
                tauri::async_runtime::spawn(perform_update(handle.clone(), false));
            }
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
