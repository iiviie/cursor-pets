// Shared oneko sprite logic: sprite-sheet layout + a small state machine that
// drives chasing / idling / sleeping. Ported and adapted from adryd325/oneko.js
// (which the sprite sheets come from) to a delta-time, config-driven model.

// Each sheet is an 8x4 grid of 32px tiles. Coordinates below are given as the
// oneko-style negative background offsets (in tile units); the source rect in
// the sheet is therefore (-col*32, -row*32).
export const TILE = 32;

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

// Deterministic-ish direction from a normalized toward-target vector.
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

export class NekoBrain {
  constructor(cfg) {
    this.cfg = cfg;
    this.x = 0;
    this.y = 0;
    this.targetX = 0;
    this.targetY = 0;
    this.state = "idle"; // idle | chase | groom | tired | sleep | wake
    this.dir = "S";
    this.idleTime = 0;
    this.frameTimer = 0;
    this.frameIndex = 0;
    this.stateTimer = 0;
    // seed for the next spontaneous idle fidget
    this.nextFidget = 3 + Math.random() * 6;
  }

  setConfig(cfg) {
    this.cfg = cfg;
  }

  place(x, y) {
    this.x = x;
    this.y = y;
    this.targetX = x;
    this.targetY = y;
  }

  setTarget(x, y) {
    this.targetX = x;
    this.targetY = y;
  }

  // Advance the simulation by dt seconds. Returns the sprite name to render.
  update(dt) {
    const dx = this.targetX - this.x;
    const dy = this.targetY - this.y;
    const dist = Math.hypot(dx, dy);
    const gap = this.cfg.follow_gap;

    const shouldChase = this.cfg.follow && dist > gap;

    if (shouldChase) {
      // Waking from sleep first flashes an "alert" beat.
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

      // Move toward the cursor, stopping at the follow gap.
      const step = Math.min(this.cfg.speed * dt, dist - gap);
      const nx = dx / dist;
      const ny = dy / dist;
      this.x += nx * step;
      this.y += ny * step;
      this.dir = directionKey(nx, ny);
      this.idleTime = 0;

      // Alternate the two walk frames.
      this.frameTimer += dt;
      if (this.frameTimer > 0.13) {
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
      const frames = SPRITES.scratchSelf.length;
      if (this.frameTimer > 0.12) {
        this.frameTimer = 0;
        this.frameIndex++;
      }
      // Groom for a couple of cycles then settle.
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
      if (this.frameTimer > 0.55) {
        this.frameTimer = 0;
        this.frameIndex ^= 1;
      }
      return "sleeping";
    }

    // Plain sitting idle, with the occasional short grooming fidget so the
    // pet feels alive without being distracting.
    this.state = "idle";
    if (this.idleTime > this.nextFidget) {
      this.nextFidget = this.idleTime + 5 + Math.random() * 8;
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

  // Resolve (sprite name, animation frames) -> the current [col,row] tile.
  currentTile(sprite) {
    const frames = SPRITES[sprite] || SPRITES.idle;
    const idx = this.frameIndex % frames.length;
    const [bx, by] = frames[idx];
    return [-bx * TILE, -by * TILE];
  }
}
