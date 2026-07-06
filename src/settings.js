import { SPRITES, TILE } from "./neko.js";

const invoke = window.__TAURI__.core.invoke;

// Must mirror Config::default() in the Rust backend.
const DEFAULTS = {
  pet: "classic",
  scale: 1.0,
  speed: 480.0,
  follow_gap: 42.0,
  follow: true,
  sleep_enabled: true,
  idle_before_sleep: 6.0,
};

const SLIDERS = {
  scale: (v) => `${v.toFixed(1)}×`,
  speed: (v) => `${Math.round(v)}`,
  follow_gap: (v) => `${Math.round(v)}px`,
  idle_before_sleep: (v) => `${Math.round(v)}s`,
};

let cfg = { ...DEFAULTS };
const sheets = {}; // pet -> Image
const previews = []; // { pet, ctx } to animate

function loadSheet(pet) {
  return new Promise((res) => {
    const img = new Image();
    img.onload = () => res(img);
    img.src = `sprites/oneko-${pet}.png`;
  });
}

function drawTile(ctx, img, spriteName, frame, size) {
  const frames = SPRITES[spriteName];
  const [bx, by] = frames[frame % frames.length];
  ctx.clearRect(0, 0, size, size);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(img, -bx * TILE, -by * TILE, TILE, TILE, 0, 0, size, size);
}

async function apply() {
  await invoke("save_config", { config: cfg });
}

function buildGrid(pets) {
  const grid = document.getElementById("pet-grid");
  grid.innerHTML = "";
  pets.forEach((pet) => {
    const card = document.createElement("div");
    card.className = "pet-card" + (pet === cfg.pet ? " selected" : "");
    card.dataset.pet = pet;

    const canvas = document.createElement("canvas");
    canvas.width = 32;
    canvas.height = 32;
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = pet;

    card.append(canvas, name);
    grid.appendChild(card);
    previews.push({ pet, ctx: canvas.getContext("2d") });

    card.addEventListener("click", () => {
      cfg.pet = pet;
      document
        .querySelectorAll(".pet-card")
        .forEach((c) => c.classList.toggle("selected", c.dataset.pet === pet));
      apply();
    });
  });
}

function bindControls() {
  for (const id of Object.keys(SLIDERS)) {
    const input = document.getElementById(id);
    input.addEventListener("input", () => {
      cfg[id] = parseFloat(input.value);
      document.getElementById(`${id}-val`).textContent = SLIDERS[id](cfg[id]);
      apply();
    });
  }
  for (const id of ["follow", "sleep_enabled"]) {
    const input = document.getElementById(id);
    input.addEventListener("change", () => {
      cfg[id] = input.checked;
      apply();
    });
  }
  document.getElementById("reset").addEventListener("click", () => {
    cfg = { ...DEFAULTS };
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
  document.getElementById("follow").checked = cfg.follow;
  document.getElementById("sleep_enabled").checked = cfg.sleep_enabled;
  document
    .querySelectorAll(".pet-card")
    .forEach((c) => c.classList.toggle("selected", c.dataset.pet === cfg.pet));
}

// Animate all previews with a gentle walk-in-place so the sheets feel alive.
let frame = 0;
let logoCtx = null;
function tick() {
  frame ^= 1;
  for (const p of previews) {
    if (sheets[p.pet]) drawTile(p.ctx, sheets[p.pet], "S", frame, 32);
  }
  if (logoCtx && sheets[cfg.pet]) drawTile(logoCtx, sheets[cfg.pet], "SE", frame, 32);
  setTimeout(tick, 280);
}

async function main() {
  try {
    cfg = { ...DEFAULTS, ...(await invoke("get_config")) };
  } catch {
    /* use defaults */
  }
  const pets = await invoke("list_pets").catch(() => Object.keys(DEFAULTS).length && ["classic"]);
  await Promise.all(pets.map(async (p) => (sheets[p] = await loadSheet(p))));

  buildGrid(pets);
  bindControls();
  syncUI();

  logoCtx = document.getElementById("logo-cat").getContext("2d");
  tick();

  // Test hook: drive a visible config change so live-apply can be verified.
  if (new URLSearchParams(location.search).has("autotest")) {
    setTimeout(async () => {
      cfg.pet = "vaporwave";
      cfg.scale = 2.5;
      syncUI();
      await apply();
    }, 1200);
  }
}

main();
