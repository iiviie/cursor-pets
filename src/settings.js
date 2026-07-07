import { SPRITES, TILE } from "./neko.js";

const invoke = window.__TAURI__.core.invoke;
const RENDER_BASE = 2;

// Must mirror Config::default() in the Rust backend.
const DEFAULTS = {
  pet: "classic",
  scale: 0.7,
  speed: 220.0,
  follow_gap: 70.0,
  reaction: 0.16,
  opacity: 1.0,
  follow: true,
  sleep_enabled: true,
  idle_before_sleep: 6.0,
  fidget_enabled: true,
};

const SLIDERS = {
  scale: (v) => `${v.toFixed(2)}×`,
  opacity: (v) => `${Math.round(v * 100)}%`,
  speed: (v) => `${Math.round(v)}`,
  follow_gap: (v) => `${Math.round(v)}px`,
  reaction: (v) => (v <= 0 ? "off" : `${Math.round(v * 1000)}ms`),
  idle_before_sleep: (v) => `${Math.round(v)}s`,
};
const TOGGLES = ["follow", "sleep_enabled", "fidget_enabled"];

let cfg = { ...DEFAULTS };
const sheets = {}; // pet -> Image

function loadSheet(pet) {
  return new Promise((res) => {
    const img = new Image();
    img.onload = () => res(img);
    img.onerror = () => res(null);
    img.src = `sprites/oneko-${pet}.png`;
  });
}

function drawTile(ctx, img, spriteName, frame, dx, dy, size) {
  const frames = SPRITES[spriteName];
  const [bx, by] = frames[frame % frames.length];
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(img, -bx * TILE, -by * TILE, TILE, TILE, dx, dy, size, size);
}

async function apply() {
  await invoke("save_config", { config: cfg });
}

/* ---------- pet swatches ---------- */
function buildSwatches(pets) {
  const grid = document.getElementById("swatches");
  grid.innerHTML = "";
  pets.forEach((pet) => {
    const btn = document.createElement("button");
    btn.className = "swatch" + (pet === cfg.pet ? " selected" : "");
    btn.dataset.pet = pet;
    const c = document.createElement("canvas");
    c.width = 32;
    c.height = 32;
    if (sheets[pet]) drawTile(c.getContext("2d"), sheets[pet], "idle", 0, 0, 0, 32);
    btn.appendChild(c);
    btn.addEventListener("click", () => selectPet(pet));
    grid.appendChild(btn);
  });
}

function selectPet(pet) {
  cfg.pet = pet;
  document
    .querySelectorAll(".swatch")
    .forEach((s) => s.classList.toggle("selected", s.dataset.pet === pet));
  document.getElementById("stage-name").textContent = pet;
  apply();
}

/* ---------- controls ---------- */
function bindControls() {
  for (const id of Object.keys(SLIDERS)) {
    const input = document.getElementById(id);
    input.addEventListener("input", () => {
      cfg[id] = parseFloat(input.value);
      document.getElementById(`${id}-val`).textContent = SLIDERS[id](cfg[id]);
      apply();
    });
  }
  for (const id of TOGGLES) {
    document.getElementById(id).addEventListener("change", (e) => {
      cfg[id] = e.target.checked;
      apply();
    });
  }
  document.getElementById("reset").addEventListener("click", () => {
    const pet = cfg.pet; // keep chosen pet on reset
    cfg = { ...DEFAULTS, pet };
    syncUI();
    apply();
  });
}

function syncUI() {
  for (const id of Object.keys(SLIDERS)) {
    const input = document.getElementById(id);
    input.value = cfg[id];
    document.getElementById(`${id}-val`).textContent = SLIDERS[id](cfg[id]);
  }
  for (const id of TOGGLES) document.getElementById(id).checked = cfg[id] !== false;
  document.getElementById("stage-name").textContent = cfg.pet;
  document
    .querySelectorAll(".swatch")
    .forEach((s) => s.classList.toggle("selected", s.dataset.pet === cfg.pet));
}

/* ---------- live stage: the selected pet paces at real size & opacity ---- */
let stageCanvas, stageCtx;
let px = 40;
let dir = 1;
let frame = 0;
let frameAcc = 0;
let lastTs = 0;

function stageStep(ts) {
  if (!lastTs) lastTs = ts;
  let dt = (ts - lastTs) / 1000;
  lastTs = ts;
  if (dt > 0.1) dt = 0.1;

  const W = stageCanvas.width;
  const H = stageCanvas.height;
  const size = Math.round(TILE * RENDER_BASE * cfg.scale);
  const paceSpeed = 34; // px/s, gentle
  px += dir * paceSpeed * dt;
  const left = size * 0.15;
  const right = W - size * 1.15;
  if (px < left) {
    px = left;
    dir = 1;
  } else if (px > right) {
    px = right;
    dir = -1;
  }
  frameAcc += dt;
  if (frameAcc > 0.16) {
    frameAcc = 0;
    frame ^= 1;
  }

  stageCtx.clearRect(0, 0, W, H);
  const img = sheets[cfg.pet];
  if (img) {
    const y = H - size - Math.round(H * 0.14);
    stageCtx.globalAlpha = cfg.opacity ?? 1;
    drawTile(stageCtx, img, dir > 0 ? "E" : "W", frame, Math.round(px), y, size);
    stageCtx.globalAlpha = 1;
  }
  requestAnimationFrame(stageStep);
}

function initStage() {
  stageCanvas = document.getElementById("stage");
  const rect = stageCanvas.getBoundingClientRect();
  stageCanvas.width = Math.max(200, Math.round(rect.width));
  stageCanvas.height = Math.max(200, Math.round(rect.height));
  stageCtx = stageCanvas.getContext("2d");
  requestAnimationFrame(stageStep);
}

async function main() {
  try {
    cfg = { ...DEFAULTS, ...(await invoke("get_config")) };
  } catch {
    /* defaults */
  }
  const pets = await invoke("list_pets").catch(() => ["classic"]);
  await Promise.all(pets.map(async (p) => (sheets[p] = await loadSheet(p))));

  buildSwatches(pets);
  bindControls();
  syncUI();
  initStage();

  if (new URLSearchParams(location.search).has("autotest")) {
    setTimeout(async () => {
      selectPet("vaporwave");
      cfg.scale = 1.6;
      cfg.opacity = 1;
      syncUI();
      await apply();
    }, 1200);
  }
}

main();
