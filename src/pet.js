import { NekoBrain, TILE } from "./neko.js";

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;

// One 32px tile is drawn at TILE * RENDER_BASE * scale pixels.
const RENDER_BASE = 2;

const canvas = document.getElementById("pet");
const ctx = canvas.getContext("2d");
ctx.imageSmoothingEnabled = false;

let cfg = null;
let brain = null;
let sprite = new Image();
let spriteReady = false;
let currentPet = null;
let renderSize = TILE * RENDER_BASE;

// Cache of the last drawn frame so we only repaint the tiny canvas when the
// sprite tile actually changes (movement itself is a cheap CSS transform).
let lastDraw = "";

function loadSprite(pet) {
  if (pet === currentPet) return;
  currentPet = pet;
  spriteReady = false;
  const img = new Image();
  img.onload = () => {
    sprite = img;
    spriteReady = true;
    lastDraw = ""; // force a repaint
  };
  img.src = `sprites/oneko-${pet}.png`;
}

function applyConfig(next) {
  cfg = next;
  if (!brain) brain = new NekoBrain(cfg);
  else brain.setConfig(cfg);
  renderSize = Math.round(TILE * RENDER_BASE * (cfg.scale || 1));
  canvas.width = renderSize;
  canvas.height = renderSize;
  ctx.imageSmoothingEnabled = false;
  loadSprite(cfg.pet);
  lastDraw = "";
}

function draw(spriteName) {
  const [sx, sy] = brain.currentTile(spriteName);
  const key = `${currentPet}:${spriteName}:${brain.frameIndex}:${renderSize}`;
  if (key !== lastDraw && spriteReady) {
    lastDraw = key;
    ctx.clearRect(0, 0, renderSize, renderSize);
    ctx.drawImage(sprite, sx, sy, TILE, TILE, 0, 0, renderSize, renderSize);
  }
  // Position: center the sprite on the pet's point.
  const px = Math.round(brain.x - renderSize / 2);
  const py = Math.round(brain.y - renderSize / 2);
  canvas.style.transform = `translate3d(${px}px, ${py}px, 0)`;
}

let lastT = 0;
function frame(t) {
  if (!lastT) lastT = t;
  let dt = (t - lastT) / 1000;
  lastT = t;
  if (dt > 0.1) dt = 0.1; // clamp after tab throttling / stalls

  if (brain) {
    const spriteName = brain.update(dt);
    draw(spriteName);
  }
  requestAnimationFrame(frame);
}

// Cursor coordinates arrive in the compositor's logical space. Map them into
// this overlay's CSS pixel space (identical when the window covers the screen
// 1:1, but this keeps us correct under any scaling surprises).
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

  // Place the pet where the cursor is right now so it doesn't dash from a corner.
  try {
    const pos = await invoke("get_cursor");
    if (pos) {
      brain.place(pos[0] * scaleX, pos[1] * scaleY);
    } else {
      brain.place(window.innerWidth / 2, window.innerHeight / 2);
    }
  } catch {
    brain.place(window.innerWidth / 2, window.innerHeight / 2);
  }

  await listen("cursor", (e) => {
    const [x, y] = e.payload;
    brain.setTarget(x * scaleX, y * scaleY);
  });

  await listen("config-changed", (e) => {
    applyConfig(e.payload);
  });

  requestAnimationFrame(frame);
}

main();
