// The memory atlas (Settings › memory, decisions 270-281): the whole memory as a cluster of stars,
// every file a planet. A small project is one system around its sun, its status; a big one is a
// galaxy of systems: its knowledge, its lessons, its decisions in runs (atlas.rs). The core holds
// what belongs to no project. A pale sun ringed in red is a status older than the last decision,
// a red planet holds a lesson given more than once, a grey rock is a note INDEX.md does not name.
// A pinch or ctrl+wheel zooms, two fingers move, a click dives in, Escape comes back, "/" searches.
// Rust reads the files; this module only draws.

const { invoke } = window.__TAURI__.core;

const SVG = "http://www.w3.org/2000/svg";
const TAU = Math.PI * 2;
const CENTER = { x: 800, y: 450 };
// The ellipse the projects sit on, around the core.
const RING = { x: 640, y: 360 };
const ZOOM_MS = 500;
const SEARCH_MS = 200;
// Planets this far apart along an orbit, orbits this far apart, in the map's units.
const PLANET_GAP = 4.6;
const ORBIT_GAP = 4.4;
// A galaxy's runs of decisions go out along two arms from its sun.
const ARM_START = 30;
const ARM_STEP = 7;
const ARM_TURN = 0.95;
// Seconds for the innermost orbit to go round once; outer ones are slower (Kepler).
const INNER_PERIOD = 24;
const HINTS = {
  root: "click a project · pinch or ctrl+wheel zoom · two fingers move · / search · esc close",
  zone: "click a system or a planet · esc back · / search",
  part: "click a planet to read it · esc back · / search",
  note: "esc back",
};

function node(tag, attributes = {}, text) {
  const element = document.createElementNS(SVG, tag);
  for (const [name, value] of Object.entries(attributes)) element.setAttribute(name, value);
  if (text !== undefined) element.textContent = text;
  return element;
}

function html(tag, className, text) {
  const element = document.createElement(tag);
  if (className) element.className = className;
  if (text !== undefined) element.textContent = text;
  return element;
}

// The same sky at every opening.
function seeded(seed) {
  let state = seed;
  return () => {
    state = (state + 0x6d2b79f5) | 0;
    let value = Math.imul(state ^ (state >>> 15), 1 | state);
    value ^= value + Math.imul(value ^ (value >>> 7), 61 | value);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

function around(center, radius, angle) {
  return { x: center.x + radius * Math.cos(angle), y: center.y + radius * Math.sin(angle) };
}

// `2026-11-06` → `06/11`.
function day(date) {
  return date ? `${date.slice(8, 10)}/${date.slice(5, 7)}` : "none";
}

function planetRadius(bytes) {
  return Math.min(3.2, 0.75 + Math.sqrt(bytes) / 45);
}

function ease(t) {
  return t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2;
}

// Notes on orbits around a star: each orbit holds as many as fit, the first notes closest.
function orbits(notes, from) {
  const rings = [];
  let radius = from;
  for (let index = 0; index < notes.length; ) {
    const room = Math.max(4, Math.floor((TAU * radius) / PLANET_GAP));
    rings.push({ radius, notes: notes.slice(index, index + room) });
    index += room;
    radius += ORBIT_GAP;
  }
  return rings;
}

// A star and its notes: the size it takes, and its orbits.
function systemOf(notes, starRadius) {
  const rings = orbits(notes, starRadius + 4);
  const reach = rings.length === 0 ? starRadius + 3 : rings[rings.length - 1].radius + 3;
  return { rings, reach };
}

export class AtlasWindow {
  constructor(dialog, { onError }) {
    this.dialog = dialog;
    this.svg = dialog.querySelector(".atlas-map");
    this.caption = dialog.querySelector(".atlas-caption");
    this.reader = dialog.querySelector(".atlas-note");
    this.title = dialog.querySelector(".atlas-cartouche-count");
    this.search = dialog.querySelector(".atlas-search");
    this.onError = onError;
    this.layout = null;
    this.focus = { zone: null, part: null };
    this.reading = false;
    this.found = null;
    this.searchTimer = null;
    this.searchRequest = 0;
    this.frame = 0;
    this.spinning = 0;
    this.selected = -1;
    this.still = matchMedia("(prefers-reduced-motion: reduce)").matches;
    dialog.addEventListener("cancel", (event) => {
      if (!this.atHome()) event.preventDefault();
    });
    dialog.addEventListener("close", () => cancelAnimationFrame(this.spinning));
    dialog.addEventListener("keydown", (event) => {
      if (this.onKey(event)) {
        event.preventDefault();
        event.stopPropagation();
      }
    });
    this.svg.addEventListener("click", (event) => {
      if (!event.target.closest(".atlas-hit")) this.back();
    });
    this.svg.addEventListener("wheel", (event) => this.onWheel(event), { passive: false });
    // WebKit's own pinch events, where the system gives them.
    let lastScale = 1;
    this.svg.addEventListener("gesturestart", (event) => {
      event.preventDefault();
      lastScale = 1;
    });
    this.svg.addEventListener("gesturechange", (event) => {
      event.preventDefault();
      this.zoomAt(this.pointOf(event), event.scale / lastScale);
      lastScale = event.scale;
    });
    this.search.addEventListener("input", () => {
      clearTimeout(this.searchTimer);
      this.searchTimer = setTimeout(() => this.runSearch(), SEARCH_MS);
    });
    this.search.addEventListener("keydown", (event) => {
      event.stopPropagation();
      if (event.key === "Escape") {
        event.preventDefault();
        this.clearSearch();
      } else if (event.key === "Enter") {
        event.preventDefault();
        this.dialog.focus();
      }
    });
  }

  async open() {
    this.focus = { zone: null, part: null };
    this.reading = false;
    this.reader.hidden = true;
    this.search.value = "";
    this.search.hidden = true;
    this.found = null;
    this.layout = null;
    this.svg.replaceChildren(node("text", { x: CENTER.x, y: CENTER.y, class: "atlas-hint", "text-anchor": "middle" }, "reading the memory…"));
    this.setView({ x: 0, y: 0, width: 1600, height: 900 });
    this.dialog.showModal();
    this.dialog.focus();
    let atlas;
    try {
      atlas = await invoke("memory_atlas");
    } catch (error) {
      this.dialog.close();
      this.onError(`${error}`);
      return;
    }
    this.layout = this.place(atlas);
    this.draw(atlas);
    this.setView(this.home());
    this.say();
    if (!this.still) this.spin();
  }

  // ----- Where everything is -----

  place(atlas) {
    const zones = atlas.systems.map((system, index) => this.placeZone(system, index));
    // Each project gets a share of the ellipse in proportion to its size.
    const total = zones.reduce((sum, zone) => sum + zone.reach * 2 + 40, 0);
    let along = 0;
    for (const zone of zones) {
      const share = zone.reach * 2 + 40;
      const angle = ((along + share / 2) / total) * TAU - Math.PI / 2;
      along += share;
      zone.center = { x: CENTER.x + RING.x * Math.cos(angle), y: CENTER.y + RING.y * Math.sin(angle) };
    }
    const coreNotes = atlas.center.parts.flatMap((part) => part.notes.map((note) => ({ ...note, kind: part.kind })));
    const core = { name: "core", hue: 45, center: CENTER, galaxy: false, sun: null, notes: coreNotes, parts: [], ...systemOf(coreNotes, 5) };
    core.system = core;
    return { zones, core, all: [...zones, core] };
  }

  placeZone(system, index) {
    // The golden angle: two projects side by side never get close colors.
    const hue = (index * 137.508 + 10) % 360;
    const zone = { name: system.name, hue, sun: system.sun, galaxy: system.galaxy, parts: [], notes: [] };
    if (!system.galaxy) {
      zone.notes = system.parts.flatMap((part) => part.notes.map((note) => ({ ...note, kind: part.kind, repeated: part.repeated })));
      Object.assign(zone, systemOf(zone.notes, 5));
      zone.system = zone;
      return zone;
    }
    // A galaxy: its parts on two arms, the first ones closest to the sun.
    let reach = 0;
    zone.parts = system.parts.map((part, order) => {
      const notes = part.notes.map((note) => ({ ...note, kind: part.kind, repeated: part.repeated }));
      const placed = { label: part.label, kind: part.kind, repeated: part.repeated, notes, zone, ...systemOf(notes, 2.6) };
      placed.system = placed;
      const distance = ARM_START + order * ARM_STEP;
      placed.offset = around({ x: 0, y: 0 }, distance, order * ARM_TURN + (order % 2) * Math.PI);
      reach = Math.max(reach, distance + placed.reach);
      return placed;
    });
    zone.reach = reach + 6;
    return zone;
  }

  // ----- Drawing -----

  draw(atlas) {
    const defs = node("defs");
    this.gradients = new Map();
    this.defs = defs;
    const sky = node("g", { class: "atlas-sky" });
    sky.append(this.stars());
    const nebulae = node("g");
    const bodies = node("g");
    this.rings = [];
    this.notes = [];
    for (const zone of this.layout.all) {
      nebulae.append(this.nebula(zone));
      bodies.append(this.drawZone(zone));
    }
    this.detail = node("g", { class: "atlas-labels" });
    for (const zone of this.layout.all) this.detail.append(...this.labels(zone));
    this.searchLayer = node("g", { class: "atlas-labels" });
    this.cursorHome = node("g");
    this.svg.replaceChildren(defs, sky, nebulae, bodies, this.detail, this.searchLayer, this.cursorHome);
    const total = this.layout.all.reduce((sum, zone) => sum + this.notesIn(zone).length + (zone.sun?.notes.length ?? 0), 0);
    this.title.textContent = `${total} files · ${atlas.systems.length} projects${atlas.warnings.length > 0 ? ` · not read: ${atlas.warnings.join(" · ")}` : ""}`;
  }

  notesIn(zone) {
    return zone.galaxy ? zone.parts.flatMap((part) => part.notes) : zone.notes;
  }

  // A sphere's shading in one hue, made once and shared.
  gradient(hue, light = 58) {
    const key = `${Math.round(hue)}-${light}`;
    if (!this.gradients.has(key)) {
      const id = `atlas-g-${this.gradients.size}`;
      const gradient = node("radialGradient", { id, cx: "0.35", cy: "0.32", r: "0.75" });
      gradient.append(
        node("stop", { offset: "0", "stop-color": `hsl(${hue} 95% ${Math.min(light + 30, 95)}%)` }),
        node("stop", { offset: "0.55", "stop-color": `hsl(${hue} 88% ${light}%)` }),
        node("stop", { offset: "1", "stop-color": `hsl(${hue} 80% ${Math.max(light - 32, 12)}%)` }),
      );
      this.defs.append(gradient);
      this.gradients.set(key, id);
    }
    return `url(#${this.gradients.get(key)})`;
  }

  glow(hue, id) {
    const gradient = node("radialGradient", { id });
    gradient.append(
      node("stop", { offset: "0", "stop-color": `hsl(${hue} 90% 60%)`, "stop-opacity": "0.34" }),
      node("stop", { offset: "0.5", "stop-color": `hsl(${hue + 30} 85% 45%)`, "stop-opacity": "0.12" }),
      node("stop", { offset: "1", "stop-color": `hsl(${hue + 60} 80% 30%)`, "stop-opacity": "0" }),
    );
    this.defs.append(gradient);
    return `url(#${id})`;
  }

  stars() {
    const random = seeded(1107);
    const stars = node("g", { class: "atlas-stars" });
    for (let index = 0; index < 700; index += 1) {
      const hue = random() < 0.3 ? 200 + random() * 120 : 30 + random() * 30;
      stars.append(
        node("circle", {
          cx: (-300 + random() * 2200).toFixed(1),
          cy: (-250 + random() * 1400).toFixed(1),
          r: (0.25 + random() ** 4 * 1.6).toFixed(2),
          fill: `hsl(${hue.toFixed(0)} 80% 85%)`,
          opacity: (0.2 + random() * 0.6).toFixed(2),
        }),
      );
    }
    return stars;
  }

  nebula(zone) {
    const group = node("g");
    const fill = this.glow(zone.hue, `atlas-n-${zone.name || "core"}`.replace(/[^\w-]/g, "_"));
    group.append(node("ellipse", { cx: zone.center.x, cy: zone.center.y, rx: zone.reach * 1.6, ry: zone.reach * 1.25, fill }));
    if (zone.galaxy) {
      // Dust along the arms.
      const random = seeded(zone.name.length * 131 + Math.round(zone.hue));
      for (let index = 0; index < zone.parts.length * 26; index += 1) {
        const order = random() * zone.parts.length;
        const arm = Math.floor(random() * 2);
        const spot = around(zone.center, ARM_START + order * ARM_STEP + (random() - 0.5) * 14, order * ARM_TURN + arm * Math.PI + (random() - 0.5) * 0.5);
        group.append(node("circle", { cx: spot.x.toFixed(1), cy: spot.y.toFixed(1), r: (0.3 + random() * 0.8).toFixed(2), fill: `hsl(${zone.hue + random() * 60 - 30} 90% 75%)`, opacity: (0.25 + random() * 0.45).toFixed(2), class: "atlas-dust" }));
      }
    }
    return group;
  }

  drawZone(zone) {
    const group = node("g", { class: "atlas-zone" });
    // The whole project, under everything else: a click anywhere on it dives in.
    group.append(this.hit(zone.center, zone.reach, () => this.zoomToZone(zone)));
    if (zone.galaxy) {
      group.append(this.sun(zone, zone.center, 6));
      for (const part of zone.parts) {
        part.center = { x: zone.center.x + part.offset.x, y: zone.center.y + part.offset.y };
        group.append(this.hit(part.center, part.reach, () => this.zoomToPart(part)));
        const hue = this.partHue(zone, part);
        const star = node("circle", { cx: part.center.x, cy: part.center.y, r: 2.6, fill: this.gradient(hue, 70), class: part.repeated > 0 ? "atlas-star repeated" : "atlas-star" });
        group.append(star, this.systemBodies(part, part.center, hue));
      }
    } else {
      group.append(this.sun(zone, zone.center, 5));
      group.append(this.systemBodies(zone, zone.center, zone.hue));
    }
    return group;
  }

  partHue(zone, part) {
    if (part.kind === "knowledge") return zone.hue + 55;
    if (part.kind === "lessons") return part.repeated > 0 ? 0 : zone.hue - 55;
    if (part.kind === "others") return zone.hue + 180;
    // Runs of decisions shift a little in hue, oldest to newest.
    return zone.hue + (zone.parts.indexOf(part) % 6) * 7 - 18;
  }

  noteHue(base, note) {
    if (note.kind === "lessons" && note.repeated > 0) return 0;
    if (note.kind === "knowledge") return base + 55;
    if (note.kind === "lessons") return base - 55;
    if (note.kind === "others") return base + 180;
    if (["global", "setup", "perso", "skills"].includes(note.kind)) return base + ["global", "setup", "perso", "skills"].indexOf(note.kind) * 70;
    return base;
  }

  // The status: bright, or pale and ringed in red when older than the last decision.
  sun(zone, center, radius) {
    const group = node("g");
    const stale = zone.sun?.stale;
    group.append(node("circle", { cx: center.x, cy: center.y, r: radius * 2.6, fill: this.glow(stale ? 30 : 48, `atlas-s-${zone.name || "core"}`.replace(/[^\w-]/g, "_")), class: "atlas-corona" }));
    const fill = stale ? "hsl(30 25% 55%)" : this.gradient(48, 72);
    const body = node("circle", { cx: center.x, cy: center.y, r: radius, fill, class: "atlas-sun" });
    group.append(body);
    if (stale) group.append(node("circle", { cx: center.x, cy: center.y, r: radius + 2.4, class: "atlas-stale-ring" }));
    const status = zone.sun?.notes ?? [];
    if (status.length > 0) {
      // The status notes ride on the sun itself, a click on it reads the newest.
      const newest = [...status].sort((one, other) => (other.date ?? "").localeCompare(one.date ?? ""))[0];
      group.append(this.hit(center, radius + 1, (event) => this.noteClick(event, newest, zone)));
    }
    return group;
  }

  systemBodies(owner, center, hue) {
    const group = node("g");
    owner.ringNodes = [];
    for (const ring of owner.rings) {
      group.append(node("circle", { cx: center.x, cy: center.y, r: ring.radius, class: "atlas-orbit" }));
      const spinner = node("g");
      const speed = TAU / (INNER_PERIOD * (ring.radius / 9) ** 1.5);
      const state = { node: spinner, center, angle: ring.radius * 1.7, speed };
      ring.notes.forEach((note, index) => {
        const at = around(center, ring.radius, (index / ring.notes.length) * TAU);
        const radius = planetRadius(note.bytes);
        const color = this.noteHue(hue, note) + ((index * 47) % 25) - 12;
        const body = note.lost ? this.rock(at, radius) : node("circle", { cx: at.x.toFixed(2), cy: at.y.toFixed(2), r: radius.toFixed(2), fill: this.gradient(color), class: "atlas-p" });
        spinner.append(body);
        const entry = { note, body, owner, at, radius, ring: state };
        this.notes.push(entry);
        const hit = this.hit(at, radius + 1.4, (event) => this.noteClick(event, note, owner));
        hit.addEventListener("mouseenter", () => this.say(`${owner.label ?? owner.name} · ${day(note.date)} · ${note.description}`));
        hit.addEventListener("mouseleave", () => this.say());
        spinner.append(hit);
      });
      group.append(spinner);
      this.rings.push(state);
      owner.ringNodes.push(state);
    }
    return group;
  }

  // A note INDEX.md does not name: a grey rock.
  rock(at, radius) {
    const random = seeded(Math.round(at.x * 31 + at.y * 17));
    const points = [0, 1, 2, 3, 4, 5].map((corner) => {
      const angle = (corner / 6) * TAU;
      const reach = radius * (0.75 + random() * 0.5);
      return `${(at.x + reach * Math.cos(angle)).toFixed(2)},${(at.y + reach * Math.sin(angle)).toFixed(2)}`;
    });
    return node("polygon", { points: points.join(" "), class: "atlas-p atlas-rock" });
  }

  // A target bigger than what it covers, invisible.
  hit(center, radius, onClick) {
    const target = node("circle", { cx: center.x, cy: center.y, r: radius, class: "atlas-hit" });
    target.addEventListener("click", (event) => {
      event.stopPropagation();
      onClick(event);
    });
    return target;
  }

  // Names drawn at the same size whatever the zoom (CSS: calc(… * var(--inv))).
  labels(zone) {
    const labels = [];
    const name = zone.name === "core" ? "CORE" : zone.name.toUpperCase();
    labels.push(node("text", { x: zone.center.x, y: zone.center.y + zone.reach + 4, class: "atlas-name", "text-anchor": "middle" }, name));
    if (zone.sun?.stale) {
      const told = zone.sun.notes.length === 0 ? "no status" : `status of ${day(zone.sun.updated)}`;
      labels.push(node("text", { x: zone.center.x, y: zone.center.y + zone.reach + 4, class: "atlas-name-under atlas-warning", "text-anchor": "middle" }, `stale: ${told} · decision ${day(zone.sun.last_decision)}`));
    }
    const repeated = zone.galaxy ? zone.parts.find((part) => part.repeated > 0)?.repeated : zone.notes.find((note) => note.repeated > 0)?.repeated;
    if (repeated) {
      const line = zone.sun?.stale ? "atlas-name-under2" : "atlas-name-under";
      labels.push(node("text", { x: zone.center.x, y: zone.center.y + zone.reach + 4, class: `${line} atlas-warning`, "text-anchor": "middle" }, `a lesson given again ×${repeated}`));
    }
    for (const part of zone.parts) {
      labels.push(node("text", { x: part.center.x, y: part.center.y + part.reach + 1, class: "atlas-part", "text-anchor": "middle" }, part.label));
    }
    return labels;
  }

  // ----- Moving -----

  spin() {
    let last = performance.now();
    const step = (now) => {
      const seconds = Math.min((now - last) / 1000, 0.1);
      last = now;
      for (const ring of this.rings) {
        ring.angle += ring.speed * seconds * 57.29578;
        ring.node.setAttribute("transform", `rotate(${ring.angle.toFixed(2)} ${ring.center.x} ${ring.center.y})`);
      }
      this.spinning = requestAnimationFrame(step);
    };
    this.spinning = requestAnimationFrame(step);
  }

  home() {
    const reaches = this.layout.all.map((zone) => ({ left: zone.center.x - zone.reach, right: zone.center.x + zone.reach, top: zone.center.y - zone.reach, bottom: zone.center.y + zone.reach + 30 }));
    // Room around for the title, the legend and the line of help, which stay on screen.
    const box = {
      left: Math.min(...reaches.map((reach) => reach.left)) - 30,
      right: Math.max(...reaches.map((reach) => reach.right)) + 30,
      top: Math.min(...reaches.map((reach) => reach.top)) - 70,
      bottom: Math.max(...reaches.map((reach) => reach.bottom)) + 90,
    };
    return this.fit({ x: (box.left + box.right) / 2, y: (box.top + box.bottom) / 2 }, box.right - box.left, box.bottom - box.top);
  }

  // The view on `center` showing at least `width` × `height`, in the screen's proportions.
  fit(center, width, height) {
    const ratio = this.svg.clientHeight > 0 ? this.svg.clientWidth / this.svg.clientHeight : 16 / 9;
    const w = Math.max(width, height * ratio);
    const h = w / ratio;
    return { x: center.x - w / 2, y: center.y - h / 2, width: w, height: h };
  }

  setView(view) {
    this.view = view;
    this.svg.setAttribute("viewBox", `${view.x} ${view.y} ${view.width} ${view.height}`);
    const pixels = this.svg.clientWidth || 1600;
    // Map units per screen pixel: what keeps names and lines the same size on screen.
    this.svg.style.setProperty("--inv", (view.width / pixels).toFixed(5));
    const close = this.layout ? this.home().width / view.width : 1;
    this.svg.classList.toggle("atlas-mid", close >= 2.2);
    this.svg.classList.toggle("atlas-near", close >= 7);
  }

  zoomTo(target) {
    const start = this.view;
    const begun = performance.now();
    const frame = ++this.frame;
    const step = (now) => {
      if (frame !== this.frame) return;
      const t = ease(Math.min((now - begun) / ZOOM_MS, 1));
      const mix = (from, to) => from + (to - from) * t;
      this.setView({ x: mix(start.x, target.x), y: mix(start.y, target.y), width: mix(start.width, target.width), height: mix(start.height, target.height) });
      if (t < 1) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }

  // The point of the map under the pointer.
  pointOf(event) {
    const box = this.svg.getBoundingClientRect();
    const scale = Math.max(this.view.width / box.width, this.view.height / box.height);
    const shownWidth = this.view.width / scale;
    const shownHeight = this.view.height / scale;
    const x = event.clientX - box.left - (box.width - shownWidth) / 2;
    const y = event.clientY - box.top - (box.height - shownHeight) / 2;
    return { x: this.view.x + x * scale, y: this.view.y + y * scale };
  }

  // Zooms by `factor` (above 1 closer) keeping `point` where it is on screen.
  zoomAt(point, factor) {
    if (!this.layout || !Number.isFinite(factor) || factor <= 0) return;
    this.frame += 1;
    const homeWidth = this.home().width;
    const width = Math.min(Math.max(this.view.width / factor, homeWidth / 400), homeWidth * 1.6);
    const applied = this.view.width / width;
    this.setView({
      x: point.x - (point.x - this.view.x) / applied,
      y: point.y - (point.y - this.view.y) / applied,
      width,
      height: this.view.height / applied,
    });
  }

  onWheel(event) {
    event.preventDefault();
    if (!this.layout) return;
    const lines = event.deltaMode === 1 ? 16 : 1;
    if (event.ctrlKey) {
      // A touchpad pinch comes as a wheel with ctrl held.
      this.zoomAt(this.pointOf(event), Math.exp((-event.deltaY * lines) / 120));
      return;
    }
    this.frame += 1;
    const perPixel = this.view.width / (this.svg.clientWidth || 1600);
    this.setView({ ...this.view, x: this.view.x + event.deltaX * lines * perPixel, y: this.view.y + event.deltaY * lines * perPixel });
  }

  atHome() {
    return !this.reading && this.focus.zone === null;
  }

  zoomToZone(zone) {
    this.focus = { zone, part: null };
    this.selected = -1;
    this.cursorHome.replaceChildren();
    this.zoomTo(this.fit(zone.center, zone.reach * 2.4, zone.reach * 2.4 + 20));
    this.say();
  }

  zoomToPart(part) {
    this.focus = { zone: part.zone, part };
    this.selected = -1;
    this.cursorHome.replaceChildren();
    this.zoomTo(this.fit(part.center, part.reach * 5, part.reach * 5));
    this.say();
  }

  // A planet far away takes the click to its system; close enough, it opens.
  noteClick(event, note, owner) {
    const zone = owner.zone ?? owner;
    const inside = owner === this.focus.part || (!zone.galaxy && this.focus.zone === zone);
    const pixels = planetRadius(note.bytes) / (this.view.width / (this.svg.clientWidth || 1600));
    if (inside || pixels >= 7) {
      this.showNote(note);
    } else if (zone.galaxy && this.focus.zone === zone && owner !== zone) {
      this.zoomToPart(owner);
    } else {
      this.zoomToZone(zone);
    }
  }

  back() {
    if (this.reading) {
      this.reading = false;
      this.reader.hidden = true;
      this.say();
    } else if (this.focus.part) {
      this.zoomToZone(this.focus.zone);
    } else if (this.focus.zone) {
      this.focus = { zone: null, part: null };
      this.selected = -1;
      this.cursorHome.replaceChildren();
      this.zoomTo(this.home());
      this.say();
    }
  }

  // ----- Keys -----

  // What the arrows go through: the projects, then a galaxy's systems, then a system's planets.
  targets() {
    if (!this.layout) return [];
    const { zone, part } = this.focus;
    if (zone === null) return this.layout.all.map((each) => ({ label: each.name, at: () => each.center, radius: each.reach, open: () => this.zoomToZone(each) }));
    if (zone.galaxy && part === null) return zone.parts.map((each) => ({ label: `${zone.name} · ${each.label}`, at: () => each.center, radius: each.reach, open: () => this.zoomToPart(each) }));
    const owner = part ?? zone;
    const notes = this.notes.filter((entry) => entry.owner === owner && (this.found === null || this.found.has(entry.note.path)));
    return notes.map((entry) => ({
      label: `${day(entry.note.date)} · ${entry.note.description}`,
      entry,
      radius: entry.radius + 1.2,
      open: () => this.showNote(entry.note),
    }));
  }

  onKey(event) {
    const key = event.key;
    if (key === "Escape") {
      if (this.atHome()) return false;
      this.back();
      return true;
    }
    if (key === "/" && this.layout && !this.reading) {
      this.search.hidden = false;
      this.search.focus();
      this.search.select();
      return true;
    }
    if (key === "+" || key === "=" || key === "-") {
      const center = { x: this.view.x + this.view.width / 2, y: this.view.y + this.view.height / 2 };
      this.zoomAt(center, key === "-" ? 1 / 1.4 : 1.4);
      return true;
    }
    const targets = this.targets();
    if (["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp"].includes(key) && targets.length > 0) {
      const step = key === "ArrowRight" || key === "ArrowDown" ? 1 : -1;
      this.selected = this.selected < 0 ? (step > 0 ? 0 : targets.length - 1) : (this.selected + step + targets.length) % targets.length;
      this.mark(targets[this.selected]);
      this.say(targets[this.selected].label);
      return true;
    }
    if (key === "Enter" && targets[this.selected]) {
      targets[this.selected].open();
      return true;
    }
    return false;
  }

  // A ring around what the arrows chose; on a planet, it goes round with it.
  mark(target) {
    this.cursorHome.replaceChildren();
    for (const entry of this.notes) entry.cursor?.remove();
    if (target.entry) {
      const { at } = target.entry;
      target.entry.cursor = node("circle", { cx: at.x, cy: at.y, r: target.radius, class: "atlas-cursor" });
      target.entry.ring.node.append(target.entry.cursor);
      return;
    }
    const at = target.at();
    this.cursorHome.append(node("circle", { cx: at.x, cy: at.y, r: target.radius, class: "atlas-cursor" }));
  }

  // ----- Reading -----

  async showNote(note) {
    this.reading = true;
    const head = html("header");
    head.append(html("h2", "", note.description), html("p", "muted", `${note.date ?? ""} · ${note.path}`));
    const body = html("pre", "", "reading…");
    this.reader.replaceChildren(head, body);
    this.reader.hidden = false;
    this.say();
    try {
      const text = await invoke("memory_note", { path: note.path });
      body.textContent = text.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, "").trim();
    } catch (error) {
      body.textContent = `${error}`;
    }
  }

  // ----- Searching -----

  async runSearch() {
    const query = this.search.value.trim();
    const request = ++this.searchRequest;
    if (query === "") {
      this.found = null;
    } else {
      let paths;
      try {
        paths = await invoke("memory_search", { query });
      } catch (error) {
        this.say(`${error}`);
        return;
      }
      if (request !== this.searchRequest) return;
      this.found = new Set(paths);
    }
    this.paintSearch();
    this.say();
  }

  clearSearch() {
    clearTimeout(this.searchTimer);
    this.searchRequest += 1;
    this.search.value = "";
    this.search.hidden = true;
    this.found = null;
    this.paintSearch();
    this.say();
    this.dialog.focus();
  }

  // Lights the planets found, dims the rest, and says how many each project and system holds.
  paintSearch() {
    const found = this.found;
    this.svg.classList.toggle("searching", found !== null);
    const counts = [];
    for (const entry of this.notes) entry.body.classList.toggle("found", found !== null && found.has(entry.note.path));
    if (found !== null) {
      for (const zone of this.layout.all) {
        const hits = this.notesIn(zone).filter((note) => found.has(note.path)).length + (zone.sun?.notes ?? []).filter((note) => found.has(note.path)).length;
        if (hits > 0) counts.push(node("text", { x: zone.center.x, y: zone.center.y - zone.reach - 2, class: "atlas-found", "text-anchor": "middle" }, `${hits} found`));
        for (const part of zone.parts) {
          const inPart = part.notes.filter((note) => found.has(note.path)).length;
          if (inPart > 0) counts.push(node("text", { x: part.center.x, y: part.center.y - part.reach, class: "atlas-found atlas-part-found", "text-anchor": "middle" }, `${inPart}`));
        }
      }
    }
    this.searchLayer.replaceChildren(...counts);
  }

  say(text) {
    if (text !== undefined) {
      this.caption.textContent = text;
      return;
    }
    const level = this.reading ? "note" : this.focus.part ? "part" : this.focus.zone ? "zone" : "root";
    const searched = this.found === null || level === "note" ? "" : ` · ${this.found.size} files hold “${this.search.value.trim()}”`;
    this.caption.textContent = `${HINTS[level]}${searched}`;
  }
}
