// Shared oneko sprite logic: sprite-sheet layout + a small state machine that
// drives chasing / idling / sleeping. Ported and adapted from adryd325/oneko.js
// (which the sprite sheets come from) to a delta-time, config-driven model with
// smoothing so it doesn't twitch when you nudge the mouse.

// Each sheet is an 8x4 grid of 32px tiles. Coordinates below are given as the
// oneko-style negative background offsets (in tile units); the source rect in
// the sheet is therefore (-col*32, -row*32).
export const TILE = 32;

// Default oneko 8x4 layout. A custom pet can override any of these (and the
// tile size / frame timing) with a manifest — see resolveManifest().
export const SPRITES = {
  idle: [[-3, -3]],
  alert: [[-7, -3]],
  tired: [[-3, -2]],
  sleeping: [[-2, 0], [-2, -1]],
  scratchSelf: [[-5, 0], [-6, 0], [-7, 0]],
  scratchWallN: [[0, 0], [0, -1]],
  scratchWallS: [[-7, -1], [-6, -2]],
  scratchWallE: [[-2, -2], [-2, -3]],
  scratchWallW: [[-4, 0], [-4, -1]],
  N: [[-1, -2], [-1, -3]],
  NE: [[0, -2], [0, -3]],
  E: [[-3, 0], [-3, -1]],
  SE: [[-5, -1], [-5, -2]],
  S: [[-6, -3], [-7, -2]],
  SW: [[-5, -3], [-6, -1]],
  W: [[-4, -2], [-4, -3]],
  NW: [[-1, 0], [-1, -1]],
};

// Direction from a normalized toward-target vector (screen y points down).
function directionKey(nx, ny) {
  let v = "";
  if (ny < -0.45) v = "N";
  else if (ny > 0.45) v = "S";
  let h = "";
  if (nx > 0.45) h = "E";
  else if (nx < -0.45) h = "W";
  const key = v + h;
  return key || (Math.abs(nx) > Math.abs(ny) ? (nx >= 0 ? "E" : "W") : (ny >= 0 ? "S" : "N"));
}

// Merge a custom pet manifest over the defaults. A manifest looks like:
//   { "tile": 32, "sprites": { "idle": [[-3,-3]], "S": [[..],[..]], ... },
//     "walkFps": 8, "sleepFps": 2 }
// Any omitted state falls back to the default oneko frame.
export function resolveManifest(manifest) {
  const m = manifest || {};
  return {
    tile: typeof m.tile === "number" && m.tile > 0 ? m.tile : TILE,
    sprites: { ...SPRITES, ...(m.sprites || {}) },
    walkFps: typeof m.walkFps === "number" && m.walkFps > 0 ? m.walkFps : 7.6,
    sleepFps: typeof m.sleepFps === "number" && m.sleepFps > 0 ? m.sleepFps : 1.8,
  };
}

export class NekoBrain {
  constructor(cfg) {
    this.cfg = cfg;
    this.anim = resolveManifest(null); // { tile, sprites, walkFps, sleepFps }
    this.x = 0; // pet position
    this.y = 0;
    this.tx = 0; // smoothed target the pet actually chases
    this.ty = 0;
    this.cursorX = 0; // latest raw cursor
    this.cursorY = 0;
    this.state = "idle"; // idle | chase | groom | tired | sleep | wake
    this.dir = "S";
    this.idleTime = 0;
    this.frameTimer = 0;
    this.frameIndex = 0;
    this.stateTimer = 0;
    this.nextFidget = 4 + Math.random() * 7;
  }

  setConfig(cfg) {
    this.cfg = cfg;
  }

  setAnim(manifest) {
    this.anim = resolveManifest(manifest);
  }

  place(x, y) {
    this.x = this.tx = this.cursorX = x;
    this.y = this.ty = this.cursorY = y;
  }

  setTarget(x, y) {
    this.cursorX = x;
    this.cursorY = y;
  }

  // Advance the simulation by dt seconds. Returns the sprite name to render.
  update(dt) {
    // Ease the chased target toward the real cursor. This is the "reaction
    // delay": with reaction > 0 the pet lags slightly and ignores tiny jitter
    // instead of snapping to every sub-pixel cursor change.
    const react = this.cfg.reaction ?? 0.15;
    const a = react <= 0 ? 1 : Math.min(1, dt / react);
    this.tx += (this.cursorX - this.tx) * a;
    this.ty += (this.cursorY - this.ty) * a;

    const dx = this.tx - this.x;
    const dy = this.ty - this.y;
    const dist = Math.hypot(dx, dy);
    const gap = this.cfg.follow_gap;
    // Hysteresis: start chasing only once clearly beyond the gap, but keep
    // going until we're back inside it. Without this the pet flickers between
    // walk and idle (looks like "running in place") right at the boundary.
    const margin = Math.max(10, gap * 0.4);
    const chasing = this.state === "chase" || this.state === "wake";
    const shouldChase =
      this.cfg.follow && (chasing ? dist > gap : dist > gap + margin);

    if (shouldChase) {
      if (this.state === "sleep" || this.state === "tired") {
        this.enter("wake");
      } else if (this.state !== "chase" && this.state !== "wake") {
        this.enter("chase");
      }
      if (this.state === "wake") {
        this.stateTimer += dt;
        if (this.stateTimer > 0.25) this.enter("chase");
        return "alert";
      }

      const step = Math.min(this.cfg.speed * dt, dist - gap);
      if (dist > 0.001) {
        const nx = dx / dist;
        const ny = dy / dist;
        this.x += nx * step;
        this.y += ny * step;
        if (step > 0.4) this.dir = directionKey(nx, ny);
      }
      this.idleTime = 0;

      this.frameTimer += dt;
      if (this.frameTimer > 1 / this.anim.walkFps) {
        this.frameTimer = 0;
        this.frameIndex ^= 1;
      }
      return this.dir;
    }

    // --- Caught up: idle / tired / sleep / occasional fidget ------------
    if (this.state === "chase" || this.state === "wake") this.enter("idle");
    this.idleTime += dt;
    this.stateTimer += dt;
    this.frameTimer += dt;

    if (this.state === "groom") {
      if (this.frameTimer > 0.12) {
        this.frameTimer = 0;
        this.frameIndex++;
      }
      if (this.stateTimer > 0.9) {
        this.frameIndex = 0;
        this.enter("idle");
      }
      return "scratchSelf";
    }

    if (this.cfg.sleep_enabled && this.idleTime > this.cfg.idle_before_sleep) {
      const sinceSleepStart = this.idleTime - this.cfg.idle_before_sleep;
      if (sinceSleepStart < 0.8) {
        this.state = "tired";
        return "tired";
      }
      this.state = "sleep";
      if (this.frameTimer > 1 / this.anim.sleepFps) {
        this.frameTimer = 0;
        this.frameIndex ^= 1;
      }
      return "sleeping";
    }

    // Plain sitting idle, with the occasional short grooming fidget.
    this.state = "idle";
    if (this.cfg.fidget_enabled !== false && this.idleTime > this.nextFidget) {
      this.nextFidget = this.idleTime + 6 + Math.random() * 9;
      this.enter("groom");
      return "scratchSelf";
    }
    return "idle";
  }

  enter(state) {
    this.state = state;
    this.stateTimer = 0;
    this.frameTimer = 0;
    this.frameIndex = 0;
  }

  // Resolve (sprite name) -> the current [srcX, srcY, tile] in the sheet.
  currentTile(sprite) {
    const map = this.anim.sprites;
    const frames = map[sprite] || map.idle || SPRITES.idle;
    const idx = this.frameIndex % frames.length;
    const [bx, by] = frames[idx];
    const t = this.anim.tile;
    return [-bx * t, -by * t, t];
  }
}
