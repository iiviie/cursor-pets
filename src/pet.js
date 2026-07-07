import { NekoBrain, TILE } from "./neko.js";

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;

// One 32px tile is drawn at TILE * RENDER_BASE * scale pixels.
const RENDER_BASE = 2;

// A single, static, full-screen canvas. The sprite is drawn at an absolute
// position and we clear only its previous rect each frame — moving a small
// transformed canvas instead leaves a trail, because webkit only damages the
// element's own rect on a transparent surface and never repaints what it
// vacated.
const canvas = document.getElementById("pet");
const ctx = canvas.getContext("2d");

let cfg = null;
let brain = null;
let sprite = new Image();
let spriteReady = false;
let currentPet = null;
let renderSize = TILE * RENDER_BASE;

// Bounds of what we last painted, so we can erase exactly that next frame.
let lastRect = null;
// Signature of the last painted frame; lets us skip idle repaints entirely.
let lastKey = "";

function resizeCanvas() {
  canvas.width = window.innerWidth;
  canvas.height = window.innerHeight;
  ctx.imageSmoothingEnabled = false;
  lastRect = null;
  lastKey = "";
}

async function loadSprite(pet) {
  if (pet === currentPet) return;
  currentPet = pet;
  spriteReady = false;
  let src;
  try {
    src = await invoke("pet_src", { id: pet });
  } catch {
    src = `sprites/oneko-${pet}.png`;
  }
  // A newer config may have swapped pets while we awaited; bail if so.
  if (pet !== currentPet) return;
  const img = new Image();
  img.onload = () => {
    if (pet !== currentPet) return;
    sprite = img;
    spriteReady = true;
    lastKey = ""; // force a repaint
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

function draw(spriteName) {
  const [sx, sy] = brain.currentTile(spriteName);
  const px = Math.round(brain.x - renderSize / 2);
  const py = Math.round(brain.y - renderSize / 2);
  const alpha = cfg.opacity ?? 1;
  const key = `${currentPet}:${spriteName}:${brain.frameIndex}:${px}:${py}:${renderSize}:${alpha}`;
  if (key === lastKey || !spriteReady) return;
  lastKey = key;

  // Clear the whole canvas, then paint the new tile. (A dirty-rect clear is
  // cheaper but webkit's damage handling on a transparent surface leaves
  // trails, so we clear everything.)
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.globalAlpha = alpha;
  ctx.drawImage(sprite, sx, sy, TILE, TILE, px, py, renderSize, renderSize);
  ctx.globalAlpha = 1;
  lastRect = { x: px, y: py, w: renderSize, h: renderSize };
}

let lastT = 0;
function frame(t) {
  if (!lastT) lastT = t;
  let dt = (t - lastT) / 1000;
  lastT = t;
  if (dt > 0.1) dt = 0.1; // clamp after stalls

  if (brain) draw(brain.update(dt));
  requestAnimationFrame(frame);
}

// Cursor coordinates arrive in the compositor's logical space. Map them into
// this overlay's CSS pixel space (identical when the window covers the screen
// 1:1, but keeps us correct under any scaling surprises).
let scaleX = 1;
let scaleY = 1;

async function main() {
  resizeCanvas();
  window.addEventListener("resize", resizeCanvas);

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
