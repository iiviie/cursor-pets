import { NekoBrain, TILE } from "./neko.js";

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;

// One tile is drawn at TILE * RENDER_BASE * scale CSS pixels.
const RENDER_BASE = 2;

// The pet is a <div> whose background is the sprite sheet. We move it with a
// transform and step frames via background-position. A moving div is an
// ordinary compositing layer the browser repaints correctly — unlike a
// transparent <canvas> on this overlay, which leaves trails (SHM renderer) or
// black boxes (DMABUF renderer) at vacated positions.
const el = document.getElementById("pet");

let cfg = null;
let brain = null;
let currentPet = null;
let sheetW = 256;
let sheetH = 128;
let sheetReady = false;
let renderSize = TILE * RENDER_BASE;

let lastKey = "";

async function loadSprite(pet) {
  if (pet === currentPet) return;
  currentPet = pet;
  sheetReady = false;

  let src;
  try {
    src = await invoke("pet_src", { id: pet });
  } catch {
    src = `sprites/oneko-${pet}.png`;
  }
  if (pet !== currentPet) return;

  let manifest = null;
  try {
    manifest = await invoke("pet_manifest", { id: pet });
  } catch {
    /* built-ins have no manifest */
  }
  if (pet !== currentPet) return;
  if (brain) brain.setAnim(manifest);

  // Load once to learn the sheet's natural size (for background scaling).
  const img = new Image();
  img.onload = () => {
    if (pet !== currentPet) return;
    sheetW = img.naturalWidth || 256;
    sheetH = img.naturalHeight || 128;
    el.style.backgroundImage = `url("${src}")`;
    sheetReady = true;
    lastKey = "";
  };
  img.src = src;
}

function applyConfig(next) {
  cfg = next;
  if (!brain) brain = new NekoBrain(cfg);
  else brain.setConfig(cfg);
  renderSize = Math.round(TILE * RENDER_BASE * (cfg.scale || 1));
  loadSprite(cfg.pet);
  lastKey = "";
}

function render(spriteName) {
  const [sx, sy, tile] = brain.currentTile(spriteName);
  const alpha = cfg.opacity ?? 1;
  const px = Math.round(brain.x - renderSize / 2);
  const py = Math.round(brain.y - renderSize / 2);

  el.style.transform = `translate3d(${px}px, ${py}px, 0)`;

  const key = `${currentPet}:${sx}:${sy}:${renderSize}:${alpha}`;
  if (key === lastKey || !sheetReady) return;
  lastKey = key;

  // Scale factor from source tile pixels to on-screen pixels.
  const k = renderSize / tile;
  el.style.width = `${renderSize}px`;
  el.style.height = `${renderSize}px`;
  el.style.opacity = alpha;
  el.style.backgroundSize = `${Math.round(sheetW * k)}px ${Math.round(sheetH * k)}px`;
  el.style.backgroundPosition = `-${Math.round(sx * k)}px -${Math.round(sy * k)}px`;
}

let lastT = 0;
function frame(t) {
  if (!lastT) lastT = t;
  let dt = (t - lastT) / 1000;
  lastT = t;
  if (dt > 0.1) dt = 0.1; // clamp after stalls

  if (brain) render(brain.update(dt));
  requestAnimationFrame(frame);
}

// Cursor coordinates arrive in the compositor's logical space. Map them into
// this overlay's CSS pixel space (identical when the window covers the screen
// 1:1, but keeps us correct under any scaling surprises).
let scaleX = 1;
let scaleY = 1;

async function main() {
  const initial = await invoke("get_config");
  applyConfig(initial);

  try {
    const [sw, sh] = await invoke("get_screen");
    if (sw > 0 && sh > 0) {
      scaleX = window.innerWidth / sw;
      scaleY = window.innerHeight / sh;
    }
  } catch {
    /* keep 1:1 */
  }

  // Start where the cursor is, so the pet doesn't dash in from a corner.
  try {
    const pos = await invoke("get_cursor");
    if (pos) brain.place(pos[0] * scaleX, pos[1] * scaleY);
    else brain.place(window.innerWidth / 2, window.innerHeight / 2);
  } catch {
    brain.place(window.innerWidth / 2, window.innerHeight / 2);
  }

  await listen("cursor", (e) => {
    const [x, y] = e.payload;
    brain.setTarget(x * scaleX, y * scaleY);
  });

  await listen("config-changed", (e) => applyConfig(e.payload));

  requestAnimationFrame(frame);
}

main();
