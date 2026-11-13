// The memory atlas (Settings › memory, decisions 270-303): the whole memory in 3D, every file a
// planet. A small project is one system around its sun, its status; a big one is a galaxy of
// systems (its knowledge, its lessons, its decisions in runs, atlas.rs) whose places draw its form.
// The core holds what belongs to no project. A pale sun is a status older than the last decision,
// a red planet with a pulsing ring holds a lesson given again, a grey lumpy rock is a file
// INDEX.md does not name. Nothing is drawn that is not a file, apart from the sky behind.
// Dragging turns the view and it keeps turning a little once let go; a pinch or the wheel goes in
// toward the pointer or out, and the view settles on a state: everything, a project, a system, a
// file. Typing searches. Rust reads the files; this module only draws.

import {
  TAU,
  PLANET_SCALE,
  START,
  seeded,
  ease,
  day,
  newest,
  buildScene,
  buildSky,
  cloudSvg,
  textureSvg,
  lavaGlowSvg,
  tilt,
  planetPosition,
  anglesOf,
  projectPoint,
  projectDirection,
  groundUnder,
  moveCenters,
} from "./atlas-scene.js";

const { invoke } = window.__TAURI__.core;

const SEARCH_MS = 200;
const FLIGHT_MS = 1600;
// A zoom gesture is over once no event came for this long.
const GESTURE_END_MS = 180;
const SPIN_MAX = 10;
const HINTS = [
  "type to search · drag or two fingers sideways to turn · pinch or wheel in and out · click a project · esc closes",
  "click a system · pinch out or esc to come back · type to search",
  "click a planet to read it · pinch out or esc to come back · type to search",
  "esc comes back",
];

// An SVG drawn once into a canvas: its filters make the noise of the textures and the clouds.
function svgCanvas(svg, width, height, done) {
  const image = new Image();
  image.onload = () => {
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    canvas.getContext("2d").drawImage(image, 0, 0, width, height);
    URL.revokeObjectURL(image.src);
    done(canvas);
  };
  image.onerror = () => URL.revokeObjectURL(image.src);
  image.src = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
}

function radialSprite(size, stops) {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const context = canvas.getContext("2d");
  const gradient = context.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  for (const [offset, color] of stops) gradient.addColorStop(offset, color);
  context.fillStyle = gradient;
  context.fillRect(0, 0, size, size);
  return canvas;
}

// Six long branches and two short ones, as on space telescope photographs.
function spikeSprite() {
  const size = 256;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const context = canvas.getContext("2d");
  context.translate(size / 2, size / 2);
  context.globalCompositeOperation = "lighter";
  const branch = (angle, length, width, alpha) => {
    context.save();
    context.rotate(angle);
    const gradient = context.createLinearGradient(0, 0, length, 0);
    gradient.addColorStop(0, `rgba(255,255,255,${alpha})`);
    gradient.addColorStop(0.25, `rgba(255,255,255,${alpha * 0.28})`);
    gradient.addColorStop(1, "rgba(255,255,255,0)");
    context.fillStyle = gradient;
    context.beginPath();
    context.moveTo(0, -width);
    context.lineTo(length, 0);
    context.lineTo(0, width);
    context.fill();
    context.restore();
  };
  for (let index = 0; index < 6; index += 1) branch((index * Math.PI) / 3 + Math.PI / 6, size / 2, 1.4, 0.9);
  branch(0, size * 0.3, 0.8, 0.5);
  branch(Math.PI, size * 0.3, 0.8, 0.5);
  const core = context.createRadialGradient(0, 0, 0, 0, 0, 14);
  core.addColorStop(0, "rgba(255,255,255,1)");
  core.addColorStop(1, "rgba(255,255,255,0)");
  context.fillStyle = core;
  context.fillRect(-14, -14, 28, 28);
  return canvas;
}

// The outline of a planet: a note INDEX.md does not name is a lumpy rock, a gas giant is flattened.
function shapePath(context, x, y, radius, planet) {
  context.beginPath();
  if (planet.type === "gas") {
    context.ellipse(x, y, radius, radius * 0.88, 0, 0, TAU);
    return;
  }
  if (planet.lost) {
    for (let index = 0; index <= 28; index += 1) {
      const angle = (index / 28) * TAU;
      const k = 1 + 0.16 * Math.sin(angle * 2 + planet.phase) + 0.09 * Math.sin(angle * 3 + planet.phase * 2.3) + 0.05 * Math.sin(angle * 7 + planet.phase * 5.1);
      const px = x + Math.cos(angle) * radius * k * 0.9;
      const py = y + Math.sin(angle) * radius * k * 0.9;
      if (index === 0) context.moveTo(px, py);
      else context.lineTo(px, py);
    }
    context.closePath();
    return;
  }
  context.arc(x, y, radius, 0, TAU);
}

// Night on the side away from the sun, darker edges, a soft highlight, a lit rim of air.
function shade(context, x, y, radius, ux, uy, planet) {
  context.save();
  shapePath(context, x, y, radius, planet);
  context.clip();
  const night = context.createLinearGradient(x + ux * radius, y + uy * radius, x - ux * radius, y - uy * radius);
  night.addColorStop(0, "rgba(0,0,0,0)");
  night.addColorStop(0.42, "rgba(0,0,0,0.05)");
  night.addColorStop(0.62, "rgba(2,1,6,0.8)");
  night.addColorStop(1, "rgba(2,1,6,0.97)");
  context.fillStyle = night;
  context.fillRect(x - radius, y - radius, radius * 2, radius * 2);
  const limb = context.createRadialGradient(x, y, radius * 0.6, x, y, radius);
  limb.addColorStop(0, "rgba(0,0,0,0)");
  limb.addColorStop(1, "rgba(0,0,0,0.5)");
  context.fillStyle = limb;
  context.fillRect(x - radius, y - radius, radius * 2, radius * 2);
  const hx = x + ux * radius * 0.55;
  const hy = y + uy * radius * 0.55;
  const spec = context.createRadialGradient(hx, hy, 0, hx, hy, radius * 0.5);
  spec.addColorStop(0, `rgba(255,250,240,${planet.type === "ocean" || planet.type === "ice" ? 0.4 : 0.22})`);
  spec.addColorStop(1, "rgba(255,250,240,0)");
  context.fillStyle = spec;
  context.fillRect(x - radius, y - radius, radius * 2, radius * 2);
  context.restore();
  const air = { gas: 0.4, rock: 0.12, ice: 0.5, lava: 0.25, ocean: 0.7 }[planet.type];
  const angle = Math.atan2(uy, ux);
  context.strokeStyle = `hsla(${planet.hue + 20},100%,80%,${air})`;
  context.lineWidth = Math.max(0.6, radius * 0.05);
  context.beginPath();
  context.arc(x, y, radius * 1.02, angle - 1.3, angle + 1.3);
  context.stroke();
}

// ----- The window -----

export class AtlasWindow {
  constructor(dialog, { onError }) {
    this.dialog = dialog;
    this.canvas = dialog.querySelector(".atlas-map");
    this.context = this.canvas.getContext("2d");
    this.caption = dialog.querySelector(".atlas-caption");
    this.reader = dialog.querySelector(".atlas-note");
    this.title = dialog.querySelector(".atlas-cartouche-count");
    this.search = dialog.querySelector(".atlas-search");
    this.onError = onError;
    this.still = matchMedia("(prefers-reduced-motion: reduce)").matches;
    this.scene = null;
    this.loop = 0;
    this.textures = new Map();
    this.sprites = new Map();
    this.glows = new Map();
    this.halos = new Map();
    this.spikes = spikeSprite();
    this.dustSprite = radialSprite(64, [[0, "rgba(2,1,5,1)"], [1, "rgba(2,1,5,0)"]]);
    this.grain = this.makeGrain();
    this.sky = buildSky(seeded(1107));
    for (const place of this.sky.cloudPlaces) svgCanvas(cloudSvg(place.hue, place.seed), 512, 512, (canvas) => this.sky.clouds.push({ ...place, canvas }));
    this.resetState();
    this.listen();
  }

  resetState() {
    this.camera = { target: { x: 0, y: 0, z: 0 }, dist: 3000, yaw: START.yaw, pitch: START.pitch };
    this.focus = { project: null, system: null, planet: null };
    this.flight = null;
    this.clock = 0;
    this.spin = { yaw: 0, pitch: 0, last: 0, wheelAt: 0 };
    this.pointer = { down: false, moved: false, x: 0, y: 0, touches: new Map(), pinch: 0 };
    this.gesture = { active: false, startDist: 0, x: 0, y: 0, timer: 0 };
    this.found = null;
    this.searchRequest = 0;
    this.drawn = [];
    this.hovered = null;
    this.selected = -1;
    this.idleSince = performance.now();
  }

  listen() {
    const { dialog, canvas, search } = this;
    dialog.addEventListener("cancel", (event) => {
      if (this.level() > 0 || this.found) event.preventDefault();
    });
    dialog.addEventListener("close", () => {
      cancelAnimationFrame(this.loop);
      this.loop = 0;
      this.resizer?.disconnect();
    });
    dialog.addEventListener("keydown", (event) => {
      if (this.onKey(event)) {
        event.preventDefault();
        event.stopPropagation();
      }
    });
    canvas.addEventListener("contextmenu", (event) => event.preventDefault());
    canvas.addEventListener("pointerdown", (event) => this.onPointerDown(event));
    canvas.addEventListener("pointermove", (event) => this.onPointerMove(event));
    canvas.addEventListener("pointerup", (event) => this.onPointerUp(event));
    canvas.addEventListener("pointercancel", (event) => this.onPointerUp(event));
    canvas.addEventListener("wheel", (event) => this.onWheel(event), { passive: false });
    // WebKit's own pinch events, where the system gives them.
    let lastScale = 1;
    canvas.addEventListener("gesturestart", (event) => {
      event.preventDefault();
      lastScale = 1;
    });
    canvas.addEventListener("gesturechange", (event) => {
      event.preventDefault();
      const box = canvas.getBoundingClientRect();
      this.zoom(lastScale / event.scale, event.clientX - box.left, event.clientY - box.top);
      lastScale = event.scale;
    });
    canvas.addEventListener("gestureend", (event) => {
      event.preventDefault();
      this.settle();
    });
    let timer = 0;
    search.addEventListener("input", () => {
      clearTimeout(timer);
      timer = setTimeout(() => this.runSearch(), SEARCH_MS);
    });
    search.addEventListener("keydown", (event) => {
      event.stopPropagation();
      if (event.key === "Escape") {
        event.preventDefault();
        this.clearSearch();
      } else if (event.key === "Enter") {
        event.preventDefault();
        this.goToFirstFound();
      }
    });
  }

  async open() {
    this.resetState();
    this.scene = null;
    this.reader.hidden = true;
    this.search.value = "";
    this.search.hidden = true;
    this.title.textContent = "";
    this.dialog.showModal();
    this.dialog.focus();
    this.fitCanvas();
    this.resizer = new ResizeObserver(() => this.fitCanvas());
    this.resizer.observe(this.canvas);
    this.say("reading the memory…");
    let atlas;
    try {
      atlas = await invoke("memory_atlas");
    } catch (error) {
      this.dialog.close();
      this.onError(`${error}`);
      return;
    }
    if (!this.dialog.open) return;
    this.scene = buildScene(atlas);
    const files = this.scene.systems.reduce((sum, system) => sum + system.planets.length + (system.status?.length ?? 0), 0);
    this.title.textContent = `${files} files · ${atlas.systems.length} projects${atlas.warnings.length > 0 ? ` · not read: ${atlas.warnings.join(" · ")}` : ""}`;
    this.say();
    let last = performance.now();
    const step = (now) => {
      const seconds = Math.min(0.05, (now - last) / 1000);
      last = now;
      this.frame(now, seconds);
      this.loop = requestAnimationFrame(step);
    };
    this.loop = requestAnimationFrame(step);
  }

  fitCanvas() {
    const box = this.canvas.getBoundingClientRect();
    this.ratio = window.devicePixelRatio || 1;
    this.width = box.width;
    this.height = box.height;
    this.lens = Math.min(box.width, box.height) * 1.1;
    this.canvas.width = Math.max(1, Math.round(box.width * this.ratio));
    this.canvas.height = Math.max(1, Math.round(box.height * this.ratio));
  }

  makeGrain() {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 160;
    const context = canvas.getContext("2d");
    const image = context.createImageData(160, 160);
    const random = seeded(77);
    for (let index = 0; index < image.data.length; index += 4) {
      const value = random() * 255;
      image.data[index] = image.data[index + 1] = image.data[index + 2] = value;
      image.data[index + 3] = 8;
    }
    context.putImageData(image, 0, 0);
    return canvas;
  }

  // ----- Camera -----

  setAngles() {
    this.angles = anglesOf(this.camera);
  }

  project(point) {
    return projectPoint(this.camera, this.angles, this, point);
  }

  projectDirection(d) {
    return projectDirection(this.angles, this, d);
  }

  groundUnder(sx, sy) {
    return groundUnder(this.camera, this, sx, sy);
  }

  level() {
    return this.focus.planet ? 3 : this.focus.system ? 2 : this.focus.project ? 1 : 0;
  }

  // Where the camera looks and how far it stands, for the state in focus.
  goal() {
    const { project, system, planet } = this.focus;
    if (planet) return { at: () => planetPosition(planet, this.clock), dist: planet.size * PLANET_SCALE * 12 + 10 };
    if (system) return { at: () => system.center, dist: Math.max(60, system.radius * 3.2) };
    if (project) return { at: () => project.center, dist: project.radius * 2.8 };
    return { at: () => ({ x: 0, y: 0, z: 0 }), dist: 3000 };
  }

  setFocus(project, system, planet) {
    this.focus = { project, system: system ?? (project && !project.galaxy ? project.systems[0] : null), planet };
    this.selected = -1;
    this.flight = { from: { target: { ...this.camera.target }, dist: this.camera.dist }, goal: this.goal(), start: performance.now() };
    if (planet) this.showNote(planet.note);
    else this.reader.hidden = true;
    this.say();
  }

  back() {
    const { project, system, planet } = this.focus;
    if (planet) this.setFocus(project, system, null);
    else if (system && project.galaxy) this.setFocus(project, null, null);
    else if (project) this.setFocus(null, null, null);
  }

  // One level down, toward what was clicked.
  enter(body) {
    const { project, system } = body;
    if (this.focus.project !== project) return this.setFocus(project, null, null);
    if (this.focus.system !== system) return this.setFocus(project, system, null);
    if (body.planet) return this.setFocus(project, system, body.planet);
    // The sun of the system in view: its newest status.
    if (body.kind === "sun" && system.status?.length > 0) this.showNote(newest(system.status));
  }

  // ----- Zooming: the camera moves while the gesture lasts, then settles on a state -----

  zoom(factor, sx = this.width / 2, sy = this.height / 2) {
    if (!this.scene || !Number.isFinite(factor) || factor <= 0) return;
    if (!this.gesture.active) Object.assign(this.gesture, { active: true, startDist: this.camera.dist });
    this.gesture.x = sx;
    this.gesture.y = sy;
    const next = Math.min(30000, Math.max(4, this.camera.dist * factor));
    const applied = next / this.camera.dist;
    if (!this.focus.planet) {
      const p = this.groundUnder(sx, sy);
      const t = this.camera.target;
      this.camera.target = { x: p.x + (t.x - p.x) * applied, y: p.y + (t.y - p.y) * applied, z: p.z + (t.z - p.z) * applied };
    }
    this.camera.dist = next;
    this.flight = null;
    clearTimeout(this.gesture.timer);
    this.gesture.timer = setTimeout(() => this.settle(), GESTURE_END_MS);
  }

  nearest(items, place, x, y) {
    let best = null;
    let bestDistance = Infinity;
    for (const item of items) {
      const q = this.project(place(item));
      if (!q) continue;
      const distance = Math.hypot(q.x - x, q.y - y);
      if (distance < bestDistance) {
        bestDistance = distance;
        best = item;
      }
    }
    return best;
  }

  // The state below, toward a point of the screen.
  enterToward(x, y) {
    const { project, system, planet } = this.focus;
    if (planet) return this.setFocus(project, system, planet);
    if (system) {
      const near = this.nearest(system.planets, (each) => planetPosition(each, this.clock), x, y);
      return this.setFocus(project, system, near);
    }
    if (project) return this.setFocus(project, this.nearest(project.systems, (each) => each.center, x, y), null);
    return this.setFocus(this.nearest(this.scene.projects, (each) => each.center, x, y), null, null);
  }

  settle() {
    clearTimeout(this.gesture.timer);
    if (!this.gesture.active) return;
    this.gesture.active = false;
    const ratio = this.camera.dist / this.gesture.startDist;
    if (ratio < 0.75) this.enterToward(this.gesture.x, this.gesture.y);
    else if (ratio > 1.33 && this.level() > 0) this.back();
    else this.setFocus(this.focus.project, this.focus.system, this.focus.planet);
  }

  // ----- Pointer and wheel -----

  // What the last drag or swipe gave: kept once let go, and slowly lost.
  push(yaw, pitch) {
    const now = performance.now();
    const seconds = Math.max(8, now - this.spin.last) / 1000;
    this.spin.yaw = Math.max(-SPIN_MAX, Math.min(SPIN_MAX, this.spin.yaw * 0.6 + (yaw / seconds) * 0.4));
    this.spin.pitch = this.spin.pitch * 0.6 + (pitch / seconds) * 0.4;
    this.spin.last = now;
  }

  turn(dx, dy) {
    this.camera.yaw -= dx * 0.0022;
    this.camera.pitch = Math.min(1.5, Math.max(0.06, this.camera.pitch + dy * 0.0018));
    this.push(-dx * 0.0022, dy * 0.0018);
  }

  onPointerDown(event) {
    this.canvas.setPointerCapture(event.pointerId);
    this.pointer.touches.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (this.pointer.touches.size === 2) {
      const [a, b] = [...this.pointer.touches.values()];
      this.pointer.pinch = Math.hypot(a.x - b.x, a.y - b.y);
    }
    Object.assign(this.pointer, { down: true, moved: false, x: event.clientX, y: event.clientY });
    this.spin = { ...this.spin, yaw: 0, pitch: 0, last: performance.now() };
    this.idleSince = performance.now();
  }

  onPointerMove(event) {
    const { pointer } = this;
    if (pointer.touches.has(event.pointerId)) pointer.touches.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (pointer.touches.size === 2) {
      const [a, b] = [...pointer.touches.values()];
      const distance = Math.hypot(a.x - b.x, a.y - b.y);
      const box = this.canvas.getBoundingClientRect();
      if (pointer.pinch) this.zoom(pointer.pinch / distance, (a.x + b.x) / 2 - box.left, (a.y + b.y) / 2 - box.top);
      pointer.pinch = distance;
      pointer.moved = true;
      return;
    }
    if (!pointer.down) {
      this.hover(this.hitTest(event.clientX, event.clientY));
      return;
    }
    const dx = event.clientX - pointer.x;
    const dy = event.clientY - pointer.y;
    if (Math.abs(dx) + Math.abs(dy) > 3) {
      pointer.moved = true;
      this.canvas.classList.add("dragging");
    }
    if (!pointer.moved) return;
    this.turn(dx, dy);
    pointer.x = event.clientX;
    pointer.y = event.clientY;
    this.idleSince = performance.now();
  }

  onPointerUp(event) {
    const { pointer } = this;
    pointer.touches.delete(event.pointerId);
    if (pointer.touches.size < 2 && pointer.pinch) {
      pointer.pinch = 0;
      this.settle();
    }
    if (pointer.touches.size > 0) return;
    this.canvas.classList.remove("dragging");
    // Held still before letting go: no momentum.
    if (performance.now() - this.spin.last > 90) this.spin = { ...this.spin, yaw: 0, pitch: 0 };
    if (pointer.down && !pointer.moved && this.scene) {
      const hit = this.hitTest(event.clientX, event.clientY);
      if (hit) this.enter(hit);
      else if (this.level() > 0) this.back();
    }
    pointer.down = false;
  }

  onWheel(event) {
    event.preventDefault();
    if (!this.scene) return;
    const lines = event.deltaMode === 1 ? 16 : 1;
    // Two fingers sideways turn the view; a pinch (ctrl) or up and down zoom.
    if (!event.ctrlKey && Math.abs(event.deltaX) > Math.abs(event.deltaY)) {
      this.camera.yaw += event.deltaX * lines * 0.0018;
      this.push(event.deltaX * lines * 0.0018, 0);
      this.spin.wheelAt = performance.now();
    } else {
      const box = this.canvas.getBoundingClientRect();
      this.zoom(Math.exp(event.deltaY * lines * (event.ctrlKey ? 0.012 : 0.0016)), event.clientX - box.left, event.clientY - box.top);
    }
    this.idleSince = performance.now();
  }

  hitTest(clientX, clientY) {
    const box = this.canvas.getBoundingClientRect();
    const x = clientX - box.left;
    const y = clientY - box.top;
    let best = null;
    for (const body of this.drawn) {
      if (Math.hypot(body.sx - x, body.sy - y) <= body.hit && (!best || body.z < best.z)) best = body;
    }
    if (best) return best;
    // Inside a system, a click near a planet is meant for it.
    if (this.level() >= 2) {
      let near = null;
      let nearDistance = 44;
      for (const body of this.drawn) {
        if (body.kind !== "planet" || body.system !== this.focus.system) continue;
        const distance = Math.hypot(body.sx - x, body.sy - y);
        if (distance < nearDistance) {
          nearDistance = distance;
          near = body;
        }
      }
      return near;
    }
    if (this.level() > 0) return null;
    for (const project of this.scene.projects) {
      const p = this.project(project.center);
      if (p && Math.hypot(p.x - x, p.y - y) < (this.lens * project.radius) / p.z) return { project, system: project.galaxy ? null : project.systems[0] };
    }
    return null;
  }

  hover(body) {
    if (body === this.hovered || (body && this.hovered && body.planet && body.planet === this.hovered.planet)) return;
    this.hovered = body;
    this.canvas.classList.toggle("pointing", body !== null);
    if (body?.planet) this.say(`${body.system.label} · ${day(body.planet.note.date)} · ${body.planet.note.description}`);
    else if (body?.kind === "sun") this.say(body.system.label);
    else this.say();
  }

  // ----- Keys -----

  // What the arrows go through: the projects, a galaxy's systems, a system's planets.
  targets() {
    if (!this.scene) return [];
    const { project, system } = this.focus;
    if (!project) return this.scene.projects.map((each) => ({ label: each.name, at: () => each.center, open: () => this.setFocus(each, null, null) }));
    if (!system) return project.systems.map((each) => ({ label: `${project.name} · ${each.label}`, at: () => each.center, open: () => this.setFocus(project, each, null) }));
    return system.planets
      .filter((each) => !this.found || this.found.has(each.note.path))
      .map((each) => ({ label: `${day(each.note.date)} · ${each.note.description}`, at: () => planetPosition(each, this.clock), open: () => this.setFocus(project, system, each) }));
  }

  onKey(event) {
    const { key } = event;
    if (key === "Escape") {
      if (this.found) {
        this.clearSearch();
        return true;
      }
      if (this.level() === 0) return false;
      this.back();
      return true;
    }
    if (!this.scene) return false;
    // Any letter or digit starts a search.
    if (key.length === 1 && /[\p{L}\p{N}]/u.test(key) && !event.ctrlKey && !event.metaKey && !event.altKey) {
      this.search.hidden = false;
      this.search.value += key;
      this.search.focus();
      this.search.dispatchEvent(new Event("input"));
      return true;
    }
    if (key === "+" || key === "=") {
      this.enterToward(this.width / 2, this.height / 2);
      return true;
    }
    if (key === "-") {
      this.back();
      return true;
    }
    if (event.shiftKey && ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(key)) {
      this.turn(key === "ArrowLeft" ? -36 : key === "ArrowRight" ? 36 : 0, key === "ArrowUp" ? 33 : key === "ArrowDown" ? -33 : 0);
      return true;
    }
    const targets = this.targets();
    if (["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp"].includes(key) && targets.length > 0) {
      const step = key === "ArrowRight" || key === "ArrowDown" ? 1 : -1;
      this.selected = this.selected < 0 ? (step > 0 ? 0 : targets.length - 1) : (this.selected + step + targets.length) % targets.length;
      this.say(targets[this.selected].label);
      return true;
    }
    if (key === "Enter" && targets[this.selected]) {
      targets[this.selected].open();
      return true;
    }
    return false;
  }

  // ----- Reading -----

  async showNote(note) {
    const head = document.createElement("header");
    const title = document.createElement("h2");
    title.textContent = note.description;
    const where = document.createElement("p");
    where.className = "muted";
    where.textContent = `${note.date ?? ""} · ${note.path}`;
    head.append(title, where);
    const body = document.createElement("pre");
    body.textContent = "reading…";
    this.reader.replaceChildren(head, body);
    this.reader.hidden = false;
    try {
      const text = await invoke("memory_note", { path: note.path });
      body.textContent = text.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, "").trim();
    } catch (error) {
      body.textContent = `${error}`;
    }
  }

  // ----- Searching: what is found stays lit, the rest goes dark -----

  async runSearch() {
    const query = this.search.value.trim();
    const request = ++this.searchRequest;
    if (query === "") {
      this.found = null;
      this.say();
      return;
    }
    let paths;
    try {
      paths = await invoke("memory_search", { query });
    } catch (error) {
      this.say(`${error}`);
      return;
    }
    if (request !== this.searchRequest) return;
    this.found = new Set(paths);
    this.say();
  }

  clearSearch() {
    this.searchRequest += 1;
    this.search.value = "";
    this.search.hidden = true;
    this.found = null;
    this.say();
    this.dialog.focus();
  }

  goToFirstFound() {
    if (!this.found || this.found.size === 0 || !this.scene) return;
    for (const system of this.scene.systems) {
      const planet = system.planets.find((each) => this.found.has(each.note.path));
      if (planet) {
        this.setFocus(planet.project, system, planet);
        this.dialog.focus();
        return;
      }
    }
  }

  foundIn(system) {
    if (!this.found) return 0;
    return system.planets.filter((planet) => this.found.has(planet.note.path)).length + (system.status ?? []).filter((note) => this.found.has(note.path)).length;
  }

  say(text) {
    if (text !== undefined) {
      this.caption.textContent = text;
      return;
    }
    const searched = this.found === null ? "" : ` · ${this.found.size} files hold “${this.search.value.trim()}”`;
    this.caption.textContent = `${HINTS[this.level()]}${searched}`;
  }

  // ----- Drawing -----

  frame(now, seconds) {
    if (!this.still) this.clock += seconds;
    const { camera, spin } = this;
    if (!this.still && !this.pointer.down && now - this.idleSince > 4000 && this.level() === 0) camera.yaw += seconds * 0.012;
    if (!this.pointer.down && now - spin.wheelAt > 60 && (Math.abs(spin.yaw) > 0.002 || Math.abs(spin.pitch) > 0.002)) {
      camera.yaw += spin.yaw * seconds;
      camera.pitch = Math.min(1.5, Math.max(0.06, camera.pitch + spin.pitch * seconds));
      spin.yaw *= Math.exp(-seconds * 0.9);
      spin.pitch *= Math.exp(-seconds * 2.5);
      this.idleSince = now;
    }
    moveCenters(this.scene, this.clock);
    if (this.flight) {
      const k = Math.min(1, (now - this.flight.start) / (this.still ? 1 : FLIGHT_MS));
      const e = ease(k);
      const to = this.flight.goal.at();
      const from = this.flight.from;
      camera.target = { x: from.target.x + (to.x - from.target.x) * e, y: from.target.y + (to.y - from.target.y) * e, z: from.target.z + (to.z - from.target.z) * e };
      camera.dist = Math.exp(Math.log(from.dist) + (Math.log(this.flight.goal.dist) - Math.log(from.dist)) * e);
      if (k >= 1) this.flight = null;
    } else if (!this.gesture.active && this.level() > 0) {
      camera.target = this.goal().at();
    }
    this.setAngles();
    const context = this.context;
    context.setTransform(this.ratio, 0, 0, this.ratio, 0, 0);
    this.drawSky();
    this.drawHalos();
    this.drawBodies(this.collect());
    this.drawLabels();
    this.drawSelection();
    const vignette = context.createRadialGradient(this.width / 2, this.height / 2, Math.min(this.width, this.height) * 0.35, this.width / 2, this.height / 2, Math.max(this.width, this.height) * 0.75);
    vignette.addColorStop(0, "rgba(0,0,0,0)");
    vignette.addColorStop(1, "rgba(0,0,0,0.55)");
    context.fillStyle = vignette;
    context.fillRect(0, 0, this.width, this.height);
    context.fillStyle = context.createPattern(this.grain, "repeat");
    context.fillRect(0, 0, this.width, this.height);
  }

  glow(hue, saturation = 100) {
    const key = `${Math.round(hue)}-${saturation}`;
    if (!this.glows.has(key)) {
      this.glows.set(key, radialSprite(160, [
        [0, "rgba(255,255,255,1)"],
        [0.12, `hsla(${hue},${saturation}%,82%,0.95)`],
        [0.3, `hsla(${hue},${saturation}%,60%,0.35)`],
        [0.6, `hsla(${hue},${saturation}%,50%,0.08)`],
        [1, `hsla(${hue},${saturation}%,50%,0)`],
      ]));
    }
    return this.glows.get(key);
  }

  drawSky() {
    const context = this.context;
    const background = context.createRadialGradient(this.width * 0.5, this.height * 0.55, 0, this.width * 0.5, this.height * 0.55, Math.max(this.width, this.height) * 0.8);
    background.addColorStop(0, "#070812");
    background.addColorStop(1, "#010103");
    context.fillStyle = background;
    context.fillRect(0, 0, this.width, this.height);
    const onScreen = (p, r) => p && p.x > -r && p.y > -r && p.x < this.width + r && p.y < this.height + r;
    context.globalCompositeOperation = "lighter";
    for (const glow of this.sky.glow) {
      const p = this.projectDirection(glow.dir);
      const r = p ? (this.lens * glow.size) / p.z : 0;
      if (!onScreen(p, r)) continue;
      context.globalAlpha = glow.alpha;
      context.drawImage(this.glow(glow.hue, 35), p.x - r, p.y - r, r * 2, r * 2);
    }
    context.globalCompositeOperation = "source-over";
    for (const dust of this.sky.dust) {
      const p = this.projectDirection(dust.dir);
      const r = p ? (this.lens * dust.size) / p.z : 0;
      if (!onScreen(p, r)) continue;
      context.globalAlpha = dust.alpha;
      context.drawImage(this.dustSprite, p.x - r, p.y - r, r * 2, r * 2);
    }
    context.globalCompositeOperation = "lighter";
    for (const cloud of this.sky.clouds) {
      const p = this.projectDirection(cloud.dir);
      if (!p) continue;
      const s = (this.lens * cloud.size) / p.z;
      context.globalAlpha = 0.35;
      context.drawImage(cloud.canvas, p.x - s, p.y - s * 0.75, s * 2, s * 1.5);
    }
    for (const batch of this.sky.batches) {
      context.globalAlpha = batch.alpha;
      context.fillStyle = batch.color;
      context.beginPath();
      for (const star of batch.stars) {
        const p = this.projectDirection(star);
        if (!p || p.x < 0 || p.y < 0 || p.x > this.width || p.y > this.height) continue;
        context.rect(p.x, p.y, batch.size, batch.size);
      }
      context.fill();
    }
    for (const star of this.sky.bright) {
      const p = this.projectDirection(star.dir);
      if (!onScreen(p, 60)) continue;
      const twinkle = 0.75 + 0.25 * Math.sin(this.clock * 1.7 + star.size);
      const r = star.size * 0.35;
      context.globalAlpha = 0.55 * twinkle;
      context.drawImage(this.glow(star.hue, 70), p.x - r, p.y - r, r * 2, r * 2);
      context.globalAlpha = 0.7 * twinkle;
      context.drawImage(this.spikes, p.x - star.size, p.y - star.size, star.size * 2, star.size * 2);
    }
    context.globalAlpha = 1;
    context.globalCompositeOperation = "source-over";
  }

  // A faint glow under each galaxy, in its plane; its form comes from where its systems are.
  drawHalos() {
    const context = this.context;
    context.globalCompositeOperation = "lighter";
    for (const project of this.scene.projects) {
      const p = this.project(project.center);
      if (!p) continue;
      const reach = project.radius * 1.25;
      const r = (this.lens * reach) / p.z;
      if (r < 3) continue;
      const fade = Math.min(1, Math.max(0, 1.5 - r / (this.width * 0.45)));
      if (fade <= 0) continue;
      if (!this.halos.has(project)) {
        const flat = project.form === "barred" || project.form === "elliptical";
        this.halos.set(project, radialSprite(512, [[0, `hsla(${project.hue + 25},90%,80%,${flat ? 0.3 : 0.35})`], [0.25, `hsla(${project.hue},90%,55%,0.12)`], [1, `hsla(${project.hue},90%,40%,0)`]]));
      }
      const turn = project.isCore ? this.clock * 0.05 : project.turn * this.clock * 0.05;
      const u = tilt(project, { x: Math.cos(turn) * reach, y: 0, z: Math.sin(turn) * reach });
      const w = tilt(project, { x: -Math.sin(turn) * reach, y: 0, z: Math.cos(turn) * reach });
      const ex = this.project({ x: project.center.x + u.x, y: project.center.y + u.y, z: project.center.z + u.z });
      const ez = this.project({ x: project.center.x + w.x, y: project.center.y + w.y, z: project.center.z + w.z });
      if (!ex || !ez) continue;
      context.globalAlpha = (project.sun?.stale ? 0.35 : 0.85) * fade * (this.focus.project === project ? 0.3 : 1);
      context.setTransform(
        (this.ratio * (ex.x - p.x)) / 256,
        (this.ratio * (ex.y - p.y)) / 256,
        (this.ratio * (ez.x - p.x)) / 256,
        (this.ratio * (ez.y - p.y)) / 256,
        this.ratio * p.x,
        this.ratio * p.y,
      );
      context.drawImage(this.halos.get(project), -256, -256);
      context.setTransform(this.ratio, 0, 0, this.ratio, 0, 0);
    }
    context.globalAlpha = 1;
    context.globalCompositeOperation = "source-over";
  }

  collect() {
    const bodies = [];
    for (const system of this.scene.systems) {
      const c = this.project(system.center);
      if (c) bodies.push({ kind: "sun", sx: c.x, sy: c.y, z: c.z, pr: (this.lens * system.sunRadius) / c.z, system, project: system.project });
      for (const planet of system.planets) {
        const q = this.project(planetPosition(planet, this.clock));
        if (!q || q.x < -60 || q.y < -60 || q.x > this.width + 60 || q.y > this.height + 60) continue;
        bodies.push({ kind: "planet", sx: q.x, sy: q.y, z: q.z, pr: (this.lens * planet.size * PLANET_SCALE) / q.z, planet, system, project: system.project, sun: c });
      }
    }
    return bodies.sort((a, b) => b.z - a.z);
  }

  fog(z) {
    return Math.max(0.35, Math.min(1, 1.5 - z / (this.camera.dist * 2.5)));
  }

  lit(body) {
    if (!this.found) return true;
    if (body.kind === "planet") return this.found.has(body.planet.note.path);
    return this.foundIn(body.system) > 0;
  }

  drawBodies(bodies) {
    const context = this.context;
    this.drawn = [];
    for (const body of bodies) {
      const lit = this.lit(body);
      const fog = this.fog(body.z) * (lit ? 1 : 0.12);
      if (body.kind === "sun") {
        this.drawSun(body, fog);
        this.drawn.push({ ...body, hit: Math.max(10, body.pr * 1.5) });
        continue;
      }
      const { planet } = body;
      if (planet.ring && body.pr >= 2) this.drawRing(body, "back", fog);
      if (body.pr < 0.9) {
        context.globalCompositeOperation = "lighter";
        context.globalAlpha = Math.min(1, Math.max(0.4, body.pr * 1.5)) * fog;
        context.fillStyle = planet.lost ? "#9a9a9a" : `hsl(${planet.repeated ? 0 : planet.hue},95%,65%)`;
        context.fillRect(body.sx, body.sy, 1.2, 1.2);
        context.globalCompositeOperation = "source-over";
      } else if (body.pr < 9) {
        const sprite = this.planetSprite(planet);
        const r = body.pr * sprite.scale;
        context.globalAlpha = fog;
        if (body.pr < 3) {
          context.drawImage(sprite.small, body.sx - r, body.sy - r, r * 2, r * 2);
        } else {
          // Big enough to see which side is lit: turn the lit side to the sun.
          const angle = body.sun ? Math.atan2(body.sun.y - body.sy, body.sun.x - body.sx) : 0;
          context.save();
          context.translate(body.sx, body.sy);
          context.rotate(angle);
          context.drawImage(sprite.canvas, -r, -r, r * 2, r * 2);
          context.restore();
        }
      } else {
        context.globalAlpha = fog;
        this.drawClose(body);
      }
      context.globalAlpha = 1;
      if (planet.ring && body.pr >= 2) this.drawRing(body, "front", fog);
      if (planet.repeated && body.pr > 0.5) {
        const pulse = 0.5 + 0.5 * Math.sin(this.clock * 3);
        context.strokeStyle = `rgba(255,70,60,${(0.4 + 0.5 * pulse) * fog})`;
        context.lineWidth = 1.5;
        context.beginPath();
        context.arc(body.sx, body.sy, Math.max(5, body.pr * 1.8) + pulse * 3, 0, TAU);
        context.stroke();
      }
      if (this.found && lit && body.pr > 0.3) {
        context.strokeStyle = "rgba(232,163,60,0.95)";
        context.lineWidth = 1.2;
        context.beginPath();
        context.arc(body.sx, body.sy, Math.max(5, body.pr * 1.5), 0, TAU);
        context.stroke();
      }
      if (this.focus.planet === planet) {
        context.strokeStyle = "rgba(232,163,60,0.9)";
        context.lineWidth = 1.5;
        context.beginPath();
        context.arc(body.sx, body.sy, Math.max(6, body.pr * 1.35), 0, TAU);
        context.stroke();
      }
      this.drawn.push({ ...body, hit: Math.max(14, body.pr * 1.4 + 4) });
    }
  }

  drawSun(body, fog) {
    const context = this.context;
    const { system } = body;
    const pale = system.stale;
    const hue = system.star === "giant" ? (system.hue * 0.15 + 12) % 360 : system.star === "dwarf" ? 210 : system.hue;
    const glow = this.glow(hue, pale ? 15 : system.star === "dwarf" ? 40 : 100);
    const r = Math.min(this.height * 0.45, Math.max(3, body.pr * 4.5));
    context.globalCompositeOperation = "lighter";
    context.globalAlpha = fog * (pale ? 0.5 : 0.95);
    context.drawImage(glow, body.sx - r, body.sy - r, r * 2, r * 2);
    if (body.pr > 0.8) {
      const k = Math.min(this.height * 0.6, Math.max(14, body.pr * 9));
      context.globalAlpha = fog * (pale ? 0.25 : 0.6);
      context.drawImage(this.spikes, body.sx - k, body.sy - k, k * 2, k * 2);
    }
    context.globalCompositeOperation = "source-over";
    if (body.pr > 6) {
      const disc = context.createRadialGradient(body.sx, body.sy, 0, body.sx, body.sy, body.pr);
      disc.addColorStop(0, "rgba(255,255,250,1)");
      disc.addColorStop(0.85, `hsla(${hue},100%,85%,1)`);
      disc.addColorStop(1, `hsla(${hue},100%,70%,0.8)`);
      context.globalAlpha = fog;
      context.fillStyle = disc;
      context.beginPath();
      context.arc(body.sx, body.sy, body.pr, 0, TAU);
      context.fill();
    }
    context.globalAlpha = 1;
  }

  drawRing(body, half, fog) {
    const context = this.context;
    const { planet } = body;
    const rx = body.pr * 2.2;
    const ry = rx * (0.18 + 0.4 * Math.sin(this.camera.pitch));
    const [from, to] = half === "back" ? [Math.PI, TAU] : [0, Math.PI];
    context.globalAlpha = fog;
    context.lineWidth = Math.max(0.6, body.pr * 0.32);
    context.strokeStyle = `hsla(${planet.hue + 35},45%,78%,0.55)`;
    context.beginPath();
    context.ellipse(body.sx, body.sy, rx, ry, planet.ringTilt, from, to);
    context.stroke();
    context.lineWidth = Math.max(0.4, body.pr * 0.1);
    context.strokeStyle = `hsla(${planet.hue + 15},40%,88%,0.4)`;
    context.beginPath();
    context.ellipse(body.sx, body.sy, rx * 1.22, ry * 1.22, planet.ringTilt, from, to);
    context.stroke();
    context.globalAlpha = 1;
  }

  // The texture of a kind of planet in a hue, made once; null until its SVG is drawn.
  texture(type, hue) {
    const bucket = (Math.round(hue / 30) * 30) % 360;
    const key = `${type}-${bucket}`;
    if (this.textures.has(key)) return this.textures.get(key);
    this.textures.set(key, null);
    svgCanvas(textureSvg(type, bucket), 400, 300, (canvas) => {
      const texture = { canvas, glow: null };
      this.textures.set(key, texture);
      // The sprites drawn without it are made again.
      for (const name of [...this.sprites.keys()]) if (name.startsWith(`${type}-${bucket}-`)) this.sprites.delete(name);
      if (type === "lava") svgCanvas(lavaGlowSvg(), 400, 300, (glow) => (texture.glow = glow));
    });
    return null;
  }

  look(planet) {
    if (planet.lost) return { ...planet, hue: 0 };
    return planet.repeated ? { ...planet, hue: 0 } : planet;
  }

  planetSprite(planet) {
    const look = this.look(planet);
    const hue = (Math.round(look.hue / 30) * 30) % 360;
    // A lumpy rock has its own outline; the others share their sprite.
    const key = `${look.type}-${hue}-${look.variant}${look.lost ? `-lost-${look.phase.toFixed(0)}` : ""}`;
    const known = this.sprites.get(key);
    if (known) return known;
    const size = 96;
    const radius = 40;
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = size;
    const context = canvas.getContext("2d");
    const texture = this.texture(look.type, hue);
    context.save();
    shapePath(context, size / 2, size / 2, radius, look);
    context.clip();
    if (texture) {
      context.drawImage(texture.canvas, look.variant * 50, 0, 300, 300, size / 2 - radius, size / 2 - radius, radius * 2, radius * 2);
      if (texture.glow) {
        context.globalCompositeOperation = "lighter";
        context.drawImage(texture.glow, look.variant * 50, 0, 300, 300, size / 2 - radius, size / 2 - radius, radius * 2, radius * 2);
      }
    } else {
      context.fillStyle = `hsl(${hue},50%,45%)`;
      context.fillRect(0, 0, size, size);
    }
    if (look.lost) {
      // Grey: a file INDEX.md does not name.
      context.globalCompositeOperation = "saturation";
      context.fillStyle = "#808080";
      context.fillRect(0, 0, size, size);
    }
    context.restore();
    shade(context, size / 2, size / 2, radius, 1, 0, look);
    // A small copy for far planets: scaling 96 px down to 4 px a thousand times a frame is slow.
    const small = document.createElement("canvas");
    small.width = small.height = 16;
    small.getContext("2d").drawImage(canvas, 0, 0, 16, 16);
    const sprite = { canvas, small, scale: size / 2 / radius };
    // Drawn without its texture: kept only until the texture comes.
    if (texture) this.sprites.set(key, sprite);
    return sprite;
  }

  // A planet seen close: its texture slides as it turns on itself.
  drawClose(body) {
    const context = this.context;
    const look = this.look(body.planet);
    const r = body.pr;
    const angle = body.sun ? Math.atan2(body.sun.y - body.sy, body.sun.x - body.sx) : -0.6;
    const texture = this.texture(look.type, (Math.round(look.hue / 30) * 30) % 360);
    const turn = this.clock * look.spin * 0.9 + look.phase;
    context.save();
    shapePath(context, body.sx, body.sy, r, look);
    context.clip();
    if (texture) {
      const scale = (r * 2) / 300;
      const width = 400 * scale;
      const shift = ((((turn * 60) % 400) + 400) % 400) * scale;
      for (let k = -1; k <= 1; k += 1) context.drawImage(texture.canvas, body.sx - r - shift + k * width, body.sy - r, width, r * 2);
      if (look.lost) {
        context.globalCompositeOperation = "saturation";
        context.fillStyle = "#808080";
        context.fillRect(body.sx - r, body.sy - r, r * 2, r * 2);
        context.globalCompositeOperation = "source-over";
      }
    } else {
      context.fillStyle = `hsl(${look.hue},50%,45%)`;
      context.fillRect(body.sx - r, body.sy - r, r * 2, r * 2);
    }
    context.restore();
    shade(context, body.sx, body.sy, r, Math.cos(angle), Math.sin(angle), look.type === "lava" ? { ...look, type: "rock" } : look);
    if (texture?.glow) {
      const scale = (r * 2) / 300;
      const width = 400 * scale;
      const shift = ((((turn * 60) % 400) + 400) % 400) * scale;
      context.save();
      shapePath(context, body.sx, body.sy, r, look);
      context.clip();
      context.globalCompositeOperation = "lighter";
      context.globalAlpha *= 0.8;
      for (let k = -1; k <= 1; k += 1) context.drawImage(texture.glow, body.sx - r - shift + k * width, body.sy - r, width, r * 2);
      context.restore();
    }
  }

  label(placed, text, x, y, color, size, under) {
    const context = this.context;
    context.font = `${size}px ${this.font()}`;
    const width = context.measureText(text).width + 8;
    const height = size + 5 + under.length * size;
    const box = { x: x - width / 2 - 6, y: y - size - 4, w: width + 12, h: height + 8 };
    if (placed.some((o) => box.x < o.x + o.w && o.x < box.x + box.w && box.y < o.y + o.h && o.y < box.y + box.h)) return;
    placed.push(box);
    context.textAlign = "center";
    context.shadowColor = "rgba(0,0,0,0.95)";
    context.shadowBlur = 6;
    context.fillStyle = color;
    context.fillText(text, x, y);
    context.font = `${size - 2}px ${this.font()}`;
    under.forEach(([line, tone], index) => {
      context.fillStyle = tone;
      context.fillText(line, x, y + (size + 1) * (index + 1));
    });
    context.shadowBlur = 0;
  }

  font() {
    this.fontFamily ??= getComputedStyle(this.dialog).getPropertyValue("--font").trim() || "monospace";
    return this.fontFamily;
  }

  drawLabels() {
    const placed = [];
    const level = this.level();
    if (level === 0) {
      // The biggest projects get their names first when two would overlap.
      const bySize = [...this.scene.projects].sort((a, b) => b.total - a.total);
      for (const project of bySize) {
        const p = this.project(project.center);
        if (!p) continue;
        const r = (this.lens * project.radius) / p.z;
        const under = [];
        if (this.found) {
          const hits = project.systems.reduce((sum, system) => sum + this.foundIn(system), 0);
          if (hits > 0) under.push([`${hits} found`, "#e8a33c"]);
        } else {
          if (project.sun?.stale) {
            const told = project.sun.notes.length === 0 ? "no status" : `status of ${day(project.sun.updated)}`;
            under.push([`stale: ${told} · decision ${day(project.sun.last_decision)}`, "#ff7a6b"]);
          }
          const again = project.systems.flatMap((system) => system.planets).filter((planet) => planet.repeated).length;
          if (again > 0) under.push([`a lesson given again`, "#ff6a5a"]);
        }
        const name = project.isCore ? "MEMORY" : `${project.name.toUpperCase()}  ${project.total}`;
        this.label(placed, name, p.x, p.y + Math.max(18, r * 0.3), `hsl(${project.hue},70%,86%)`, 11, under);
      }
    } else if (this.focus.project?.galaxy) {
      const systems = [...this.focus.project.systems].sort((a, b) => (this.project(a.center)?.z ?? 1e9) - (this.project(b.center)?.z ?? 1e9));
      for (const system of systems) {
        if (level >= 2 && system !== this.focus.system) continue;
        const p = this.project(system.center);
        if (!p) continue;
        const under = this.found ? (this.foundIn(system) > 0 ? [[`${this.foundIn(system)} found`, "#e8a33c"]] : []) : system.stale ? [["stale", "#ff7a6b"]] : system.repeated > 0 ? [["a lesson given again", "#ff6a5a"]] : [];
        this.label(placed, `${system.label} · ${system.planets.length || system.status?.length || 0}`, p.x, p.y + Math.max(16, (this.lens * system.sunRadius * 2.2) / p.z), `hsl(${system.hue},90%,78%)`, 11, under);
      }
    }
    if (this.still) this.label(placed, "motion reduced by the system: orbits stopped", this.width / 2, this.height - 40, "#e07a3c", 11, []);
  }

  // A ring around what the arrows chose.
  drawSelection() {
    const target = this.targets()[this.selected];
    if (!target) return;
    const p = this.project(target.at());
    if (!p) return;
    const context = this.context;
    context.strokeStyle = "#fff6c8";
    context.lineWidth = 2;
    context.beginPath();
    context.arc(p.x, p.y, 16, 0, TAU);
    context.stroke();
  }
}
