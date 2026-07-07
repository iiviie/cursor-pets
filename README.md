# cursor-pet

A small desktop pet that follows your cursor. It is a retro pixel cat (or dog)
that walks toward your mouse, sits and idles when it catches up, and curls up to
sleep when you leave it alone. Built with Rust and Tauri so it stays light.

![demo](docs/demo.gif)

The clip shows the pet following the cursor and the settings window. There is
also a [higher quality version](docs/demo.mp4). The settings window on its own:

![settings](docs/settings.png)

## What it does

- Follows your cursor with animated pixel sprites, then settles into an idle when it arrives.
- Stays out of the way. The overlay is click-through and never takes focus, so it sits on top of your work without blocking anything. No blur, no dimming.
- Idles and sleeps. It does small idle movements and falls asleep after a while, then wakes up when you move the mouse.
- Ships with 5 pets (classic cat, dog, maia, tora, vaporwave), and you can add your own sprite sheet.
- Has a settings window for the pet, size, opacity, speed, follow distance, smoothness, and sleep timing. Changes apply right away and are saved to disk.
- Lives in the system tray. From there you can open settings, hide the pet, check for updates, or quit.
- Can start on login (Linux, Windows, and macOS).

## Install

Download the file for your system from the
[Releases](https://github.com/iiviie/cursor-pets/releases) page.

| System | File |
| --- | --- |
| Linux (Arch and others) | `cursor-pet_*_amd64.AppImage`. Make it executable (`chmod +x`) and run it. |
| Debian / Ubuntu / Fedora | the `.deb` or `.rpm`, which pulls in `webkit2gtk` for you. |
| Windows | `cursor-pet_*_x64-setup.exe` (or the `.msi`). |
| macOS | `cursor-pet_*_universal.dmg` (Apple Silicon and Intel). |

The builds are not code-signed, so your OS shows a one-time warning the first
time you open it:

- Windows: SmartScreen, click "More info" then "Run anyway".
- macOS: right-click the app, then "Open", then "Open". Or run `xattr -dr com.apple.quarantine /Applications/cursor-pet.app`.

Once it starts, it lives in the system tray. There is no window until you open
settings (left-click the tray icon, or right-click for the menu).

### Updating

The app checks for a newer version on launch and installs it in the background,
which takes effect the next time you start it. You can also run "Check for
updates" from the tray menu.

## Build from source

You do not need Node or the Tauri CLI. The frontend is plain static HTML, CSS,
and JS embedded at build time, so a normal `cargo` build produces the whole app.

```bash
cd src-tauri
cargo run --release        # run it
cargo build --release      # or just build ./target/release/cursor-pet
```

You need Rust (1.77+) and the system libraries Tauri uses (`webkit2gtk-4.1` and
GTK dev packages).

## How it works

One process runs everything. A fullscreen, transparent, click-through window
draws the pet, and the settings window is created only when you open it and
destroyed when you close it, so nothing heavy sits idle in the background.

The Rust side reads the global cursor position and sends it to the overlay only
when it changes, so an idle desktop sends nothing. The overlay runs the movement
and animation. The pet itself is a `div` with the sprite sheet as its background,
moved with a CSS transform, the same approach oneko.js uses. (A transparent
`<canvas>` on this overlay left trails or black boxes depending on the WebKit
renderer, so the div is both simpler and cleaner.)

## Footprint

Measured on Linux, release build, idle, overlay only. These are real PSS numbers
(shared memory counted once, not the inflated RSS figure).

| Process | Memory |
| --- | --- |
| cursor-pet (main) | ~113 MB |
| WebKitWebProcess | ~121 MB |
| WebKitNetworkProcess | ~39 MB |
| Total | ~274 MB |

That is the WebKit floor for a webview app, roughly half of a comparable
Electron app. It is not tiny. The settings window costs nothing while closed,
and idle CPU is close to zero because events only fire when the cursor moves.
The binary on disk is about 5 MB.

## Configuration

Settings are saved to `~/.config/dev.crosmos.cursorpet/config.json`.

| Field | Meaning |
| --- | --- |
| `pet` | which sprite sheet to use, a built-in or a custom id |
| `scale` | rendered size |
| `opacity` | sprite opacity, 0 to 1 |
| `speed` | chase speed in logical pixels per second |
| `follow_gap` | how far from the cursor the pet stops |
| `reaction` | follow smoothing in seconds; higher is calmer and ignores small jitter |
| `follow` | whether the pet chases the cursor |
| `sleep_enabled` | whether it sleeps when idle |
| `idle_before_sleep` | seconds of idle before it sleeps |
| `fidget_enabled` | whether it does occasional idle movements |

Values are clamped on load, so a hand-edited config cannot break the pet.

## Custom pets

Click the `+` tile in the settings window to import your own sprite sheet. It
should be an 8x4 grid of 32px tiles in the oneko layout (at least 256x128, with
both dimensions a multiple of 32). It is copied into the app data folder and
shows up next to the built-in pets. The `x` on a custom pet removes it.

A custom pet can include an optional manifest (`<sheet>.json` next to the PNG at
import time) to override the tile size, the frame map per state, and the
walk/sleep animation speed. Any state you leave out falls back to the default
oneko frames. See [docs/example-pet-manifest.json](docs/example-pet-manifest.json).

## Platforms

Written to run on Linux, Windows, and macOS. The platform-specific parts (the
global cursor and overlay placement) live behind one small abstraction in
`src-tauri/src/cursor.rs`.

| Platform | State | Notes |
| --- | --- | --- |
| Linux / Hyprland | tested | Reads the cursor from Hyprland's IPC. Window rules float, pin, and un-blur the overlay. |
| Windows | built, needs real-world testing | Cursor via `GetCursorPos`, a native always-on-top transparent window. |
| macOS | built, needs real-world testing | Cursor via Core Graphics. Needs Accessibility permission to read the cursor. |
| Other Linux desktops | partial | Cursor via `device_query` (X11 or XWayland). A pure Wayland session without XWayland has no portable way to read the global cursor. |

Multi-monitor and fractional scaling past the primary output are still to do.

## Releasing (maintainers)

Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds the
installers for all three systems, signs the updater files, generates
`latest.json`, and attaches everything to a draft GitHub Release.

Add one repository secret (Settings, then Secrets and variables, then Actions):

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | contents of the updater private key, kept locally and never committed |

Do not add a `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secret. This key has no
password, and GitHub cannot store an empty secret. The workflow references it,
but an unset secret resolves to an empty string, which the signer accepts.

To cut a release, bump `version` in `src-tauri/tauri.conf.json`, then:

```bash
git tag v0.1.0
git push origin v0.1.0
```

When CI finishes, open the draft release, review it, and publish. Publishing
makes `latest.json` live so running apps pick up the update on their next start.

## Credits

- Sprite sheets are from the oneko project, the classic cursor-chasing neko,
  bundled via [kyrie25/spicetify-oneko](https://github.com/kyrie25/spicetify-oneko).
  The original oneko.js is by [adryd325](https://github.com/adryd325/oneko.js).
- Built with [Tauri](https://tauri.app).
