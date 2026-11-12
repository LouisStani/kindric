// The memory atlas (Settings › memory, decisions 270-278): the memory drawn as a star chart. Each
// project is a system whose sun is its status and whose planets are its decisions, knowledge and
// lessons; what belongs to no project is the core. A pale sun is a status older than the last
// decision, a red planet holds a lesson given more than once, an asteroid is a note the zone's
// INDEX.md does not name. A click zooms: a system, then a planet whose notes come out around it,
// then a note, shown to read. Escape or a click on the sky goes back one level. Rust reads the
// files (atlas.rs); this module only draws.

const { invoke } = window.__TAURI__.core;

const SVG = "http://www.w3.org/2000/svg";
const WIDTH = 1600;
const HEIGHT = 900;
const CENTER = { x: 800, y: 470 };
// The ellipse the systems sit on, around the core.
const ORBIT = { x: 600, y: 300 };
// Where each planet sits around its sun, and how far.
const PLANETS = {
  decisions: { angle: -35, distance: 30 },
  knowledge: { angle: 110, distance: 22 },
  lessons: { angle: 215, distance: 40 },
};
// Width of the view on a planet, in the chart's units; its notes fill most of its height.
const PLANET_VIEW = 120;
const ZOOM_MS = 450;
const HINTS = {
  galaxy: "click a system · esc close",
  system: "click a planet, or the sun for the status · esc back",
  planet: "click a note to read it · esc back",
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

function titled(element, text) {
  element.append(node("title", {}, text));
  return element;
}

// The same sky at every opening: a small seeded generator.
function seeded(seed) {
  let state = seed;
  return () => {
    state = (state + 0x6d2b79f5) | 0;
    let value = Math.imul(state ^ (state >>> 15), 1 | state);
    value ^= value + Math.imul(value ^ (value >>> 7), 61 | value);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

function polar(center, radiusX, radiusY, degrees) {
  const radians = (degrees * Math.PI) / 180;
  return { x: center.x + radiusX * Math.cos(radians), y: center.y + radiusY * Math.sin(radians) };
}

function planetOf(system, name) {
  return system.planets.find((planet) => planet.name === name) ?? { name, notes: [], repeated: 0 };
}

function notesOf(system) {
  return system.planets.reduce((sum, planet) => sum + planet.notes.length, (system.sun?.notes.length ?? 0) + system.others.length);
}

function lostOf(system) {
  return [...system.planets.flatMap((planet) => planet.notes), ...(system.sun?.notes ?? []), ...system.others].filter((note) => note.lost);
}

// `2026-11-06` → `06/11`.
function day(date) {
  return date ? `${date.slice(8, 10)}/${date.slice(5, 7)}` : "none";
}

// A view of `width` centered on `point`, with the chart's proportions.
function viewAround(point, width) {
  const height = (width * HEIGHT) / WIDTH;
  return { x: point.x - width / 2, y: point.y - height / 2, width, height };
}

function ease(t) {
  return t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2;
}

export class AtlasWindow {
  constructor(dialog, { onError }) {
    this.dialog = dialog;
    this.svg = dialog.querySelector(".atlas-map");
    this.caption = dialog.querySelector(".atlas-caption");
    this.reader = dialog.querySelector(".atlas-note");
    this.onError = onError;
    this.atlas = null;
    // "galaxy", "system", "planet" or "note"; what is zoomed on.
    this.level = "galaxy";
    this.focus = { system: null, planet: null };
    this.view = viewAround({ x: WIDTH / 2, y: HEIGHT / 2 }, WIDTH);
    this.frame = 0;
    // Escape goes back one level; from the whole chart, it closes.
    dialog.addEventListener("cancel", (event) => {
      if (this.level === "galaxy") return;
      event.preventDefault();
      this.back();
    });
    this.svg.addEventListener("click", (event) => {
      if (!event.target.closest(".atlas-target")) this.back();
    });
    // The keyboard does what the mouse does: arrows choose, Enter zooms or opens.
    this.selected = -1;
    this.noteTargets = [];
    dialog.addEventListener("keydown", (event) => {
      if (this.onKey(event.key)) {
        event.preventDefault();
        event.stopPropagation();
      }
    });
  }

  async open() {
    this.level = "galaxy";
    this.svg.setAttribute("class", "atlas-map");
    this.focus = { system: null, planet: null };
    this.reader.hidden = true;
    this.setView(viewAround({ x: WIDTH / 2, y: HEIGHT / 2 }, WIDTH));
    this.svg.replaceChildren(node("text", { x: WIDTH / 2, y: HEIGHT / 2, class: "atlas-hint", "text-anchor": "middle" }, "reading the memory…"));
    this.dialog.showModal();
    this.dialog.focus();
    try {
      this.atlas = await invoke("memory_atlas");
    } catch (error) {
      this.dialog.close();
      this.onError(`${error}`);
      return;
    }
    this.draw();
    this.say();
  }

  say(text) {
    this.caption.textContent = text ?? HINTS[this.level];
  }

  setView(view) {
    this.view = view;
    this.svg.setAttribute("viewBox", `${view.x} ${view.y} ${view.width} ${view.height}`);
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

  setLevel(level) {
    this.svg.classList.remove(`level-${this.level}`);
    this.level = level;
    this.svg.classList.add(`level-${level}`);
    this.selected = -1;
    this.cursorLayer?.replaceChildren();
    this.say();
  }

  // What the arrows go through at this level: where it is, how big, what Enter does.
  targets() {
    if (this.level === "galaxy") {
      return (this.layouts ?? []).map((layout) => ({ at: layout.place, radius: layout.reach + 10, label: layout.name, open: () => this.showSystem(layout) }));
    }
    if (this.level === "system") {
      const layout = this.focus.system;
      const bodies = layout.sun ? [layout.sun, ...layout.planets] : layout.planets;
      const unit = this.view.width / WIDTH;
      return bodies.map((body) => ({ at: body.at, radius: body.radius + 5 * unit, label: `${body.planet.name} · ${body.planet.notes.length} notes`, open: () => this.showPlanet(layout, body) }));
    }
    if (this.level === "planet") return this.noteTargets;
    return [];
  }

  onKey(key) {
    // Here rather than in "cancel": a dialog lets its close be refused only once between two user
    // activations, and a second Escape in a row would close the atlas instead of going back.
    if (key === "Escape" && this.level !== "galaxy") {
      this.back();
      return true;
    }
    const targets = this.targets();
    if ((key === "ArrowRight" || key === "ArrowDown" || key === "ArrowLeft" || key === "ArrowUp") && targets.length > 0) {
      const step = key === "ArrowRight" || key === "ArrowDown" ? 1 : -1;
      this.selected = this.selected < 0 ? (step > 0 ? 0 : targets.length - 1) : (this.selected + step + targets.length) % targets.length;
      const target = targets[this.selected];
      this.cursorLayer.replaceChildren(node("circle", { cx: target.at.x, cy: target.at.y, r: target.radius, class: "atlas-cursor" }));
      this.say(target.label);
      return true;
    }
    if (key === "Enter" && this.selected >= 0 && targets[this.selected]) {
      targets[this.selected].open();
      return true;
    }
    return false;
  }

  back() {
    if (this.level === "note") {
      this.reader.hidden = true;
      this.setLevel("planet");
    } else if (this.level === "planet") {
      this.notesLayer.replaceChildren();
      this.showSystem(this.focus.system);
    } else if (this.level === "system") {
      this.detailLayer.replaceChildren();
      this.focus = { system: null, planet: null };
      this.setLevel("galaxy");
      this.zoomTo(viewAround({ x: WIDTH / 2, y: HEIGHT / 2 }, WIDTH));
    }
  }

  draw() {
    const atlas = this.atlas;
    const sky = node("g");
    sky.append(this.chart(), this.stars());
    const systems = node("g");
    const biggest = Math.max(1, ...atlas.systems.map(notesOf));
    const mostDecisions = Math.max(1, ...atlas.systems.map((system) => planetOf(system, "decisions").notes.length));
    this.layouts = atlas.systems.map((system, index) => {
      const angle = (index / atlas.systems.length) * 360 - 90;
      const place = polar(CENTER, ORBIT.x, ORBIT.y, angle);
      sky.append(node("line", { x1: CENTER.x, y1: CENTER.y, x2: place.x, y2: place.y, class: "atlas-thread" }));
      const layout = this.layout(system, place, notesOf(system) / biggest, mostDecisions);
      systems.append(this.system(layout));
      return layout;
    });
    const core = this.coreLayout(atlas.center);
    this.layouts.push(core);
    this.detailLayer = node("g", { class: "atlas-detail" });
    this.notesLayer = node("g", { class: "atlas-notes" });
    this.cursorLayer = node("g");
    this.svg.replaceChildren(sky, this.core(core), systems, this.detailLayer, this.notesLayer, this.cursorLayer, this.cartouche(atlas), this.legend());
    this.svg.classList.add("level-galaxy");
  }

  // Where a system's bodies are, in the chart's units: what the drawing and the zoom share.
  layout(system, place, size, mostDecisions) {
    const scale = 0.9 + size * 0.6;
    const planets = Object.entries(PLANETS).map(([name, { angle, distance }]) => {
      const planet = planetOf(system, name);
      const count = planet.notes.length;
      const radius = name === "decisions" ? 2 + 7 * Math.sqrt(count / mostDecisions) : 2 + Math.min(count, 16) ** 0.5;
      const at = polar(place, distance * scale, distance * scale, angle);
      return { planet, at, radius, orbit: distance * scale };
    });
    const sun = { planet: { name: "status", notes: system.sun?.notes ?? [], repeated: 0 }, at: place, radius: 5 };
    return { system, name: system.name, place, scale, planets, sun, reach: 46 * scale };
  }

  coreLayout(center) {
    const planets = center.planets.map((planet, index) => ({
      planet,
      at: polar(CENTER, 26, 26, index * 90 - 45),
      radius: 2 + Math.sqrt(planet.notes.length),
      orbit: 26,
    }));
    return { system: center, name: "core", place: CENTER, scale: 1, planets, sun: null, reach: 46, core: true };
  }

  // The plate: its frame, graduations, and a polar grid around the core.
  chart() {
    const chart = node("g", { class: "atlas-chart" });
    chart.append(node("rect", { x: 14, y: 14, width: WIDTH - 28, height: HEIGHT - 28, class: "atlas-frame" }));
    chart.append(node("rect", { x: 24, y: 24, width: WIDTH - 48, height: HEIGHT - 48, class: "atlas-frame thin" }));
    for (let x = 24; x <= WIDTH - 24; x += 16) {
      for (const [edge, toward] of [[24, 1], [HEIGHT - 24, -1]]) {
        chart.append(node("line", { x1: x, y1: edge, x2: x, y2: edge + toward * 4, class: "atlas-tick" }));
      }
    }
    for (let y = 24; y <= HEIGHT - 24; y += 16) {
      for (const [edge, toward] of [[24, 1], [WIDTH - 24, -1]]) {
        chart.append(node("line", { x1: edge, y1: y, x2: edge + toward * 4, y2: y, class: "atlas-tick" }));
      }
    }
    // Right ascension along the top, every 2 hours; declination down the side, every 30°.
    for (let hour = 0; hour <= 24; hour += 2) {
      const x = 24 + (hour / 24) * (WIDTH - 48);
      chart.append(node("line", { x1: x, y1: 24, x2: x, y2: 34, class: "atlas-tick" }));
      chart.append(node("line", { x1: x, y1: HEIGHT - 24, x2: x, y2: HEIGHT - 34, class: "atlas-tick" }));
      if (hour > 0 && hour < 24) chart.append(node("text", { x, y: 46, class: "atlas-scale", "text-anchor": "middle" }, `${hour}h`));
    }
    for (let degrees = 60; degrees >= -60; degrees -= 30) {
      const y = 24 + ((90 - degrees) / 180) * (HEIGHT - 48);
      chart.append(node("line", { x1: 24, y1: y, x2: 34, y2: y, class: "atlas-tick" }));
      chart.append(node("line", { x1: WIDTH - 24, y1: y, x2: WIDTH - 34, y2: y, class: "atlas-tick" }));
      chart.append(node("text", { x: 40, y: y + 3, class: "atlas-scale" }, `${degrees > 0 ? "+" : ""}${degrees}°`));
    }
    for (let ring = 1; ring <= 5; ring += 1) {
      chart.append(node("ellipse", { cx: CENTER.x, cy: CENTER.y, rx: ring * 150, ry: ring * 80, class: "atlas-grid" }));
    }
    for (let degrees = 0; degrees < 360; degrees += 30) {
      const end = polar(CENTER, 760, 420, degrees);
      chart.append(node("line", { x1: CENTER.x, y1: CENTER.y, x2: end.x, y2: end.y, class: "atlas-grid" }));
    }
    return chart;
  }

  stars() {
    const random = seeded(1107);
    const stars = node("g", { class: "atlas-stars" });
    for (let index = 0; index < 320; index += 1) {
      const radius = 0.3 + random() ** 3 * 1.4;
      stars.append(node("circle", { cx: 30 + random() * (WIDTH - 60), cy: 30 + random() * (HEIGHT - 60), r: radius.toFixed(2), opacity: (0.15 + random() * 0.45).toFixed(2) }));
    }
    return stars;
  }

  // What belongs to no project: a small cluster at the center.
  core(layout) {
    const core = node("g", { class: "atlas-core atlas-target" });
    core.append(node("circle", { cx: CENTER.x, cy: CENTER.y, r: 46, class: "atlas-halo" }));
    core.append(node("circle", { cx: CENTER.x, cy: CENTER.y, r: 3.5, class: "atlas-sun" }));
    for (const [index, body] of layout.planets.entries()) {
      core.append(this.body(layout, body, "atlas-planet core"));
      const label = polar(CENTER, 40, 40, index * 90 - 45);
      core.append(node("text", { x: label.x, y: label.y + 3, class: "atlas-small", "text-anchor": label.x < CENTER.x ? "end" : "start" }, body.planet.name));
    }
    core.append(node("text", { x: CENTER.x, y: CENTER.y + 66, class: "atlas-name", "text-anchor": "middle" }, "CORE"));
    core.addEventListener("click", (event) => this.onSystemClick(event, layout));
    layout.group = core;
    return core;
  }

  system(layout) {
    const { system, place, reach } = layout;
    const group = node("g", { class: "atlas-system atlas-target" });
    for (const body of layout.planets) {
      group.append(node("circle", { cx: place.x, cy: place.y, r: body.orbit.toFixed(1), class: "atlas-orbit" }));
    }
    group.append(this.sun(layout, reach + 46));
    for (const body of layout.planets) group.append(this.planet(layout, body));
    const lost = lostOf(system);
    if (lost.length > 0) group.append(this.asteroids(lost, place, reach + 10));
    const counts = layout.planets.map((body) => body.planet.notes.length);
    group.append(node("text", { x: place.x, y: place.y + reach + 22, class: "atlas-name", "text-anchor": "middle" }, system.name.toUpperCase()));
    group.append(node("text", { x: place.x, y: place.y + reach + 34, class: "atlas-small", "text-anchor": "middle" }, `${counts[0]} d · ${counts[1]} k · ${counts[2]} l`));
    group.addEventListener("click", (event) => this.onSystemClick(event, layout));
    layout.group = group;
    return group;
  }

  // A body a click on a zoomed system opens: a planet, or the sun with the status notes.
  body(layout, body, classes) {
    const circle = node("circle", { cx: body.at.x.toFixed(1), cy: body.at.y.toFixed(1), r: body.radius.toFixed(1), class: classes });
    titled(circle, `${body.planet.name} · ${body.planet.notes.length}`);
    circle.addEventListener("click", (event) => {
      if (this.level !== "system" || this.focus.system !== layout) return;
      event.stopPropagation();
      this.showPlanet(layout, body);
    });
    return circle;
  }

  // The status. Pale when it is older than the last decision: it no longer tells where things are.
  // The warning goes under the system's name, at `below`.
  sun(layout, below) {
    const sun = layout.system.sun;
    const { x, y } = layout.place;
    const group = node("g", { class: sun.stale ? "atlas-star stale" : "atlas-star" });
    group.append(node("circle", { cx: x, cy: y, r: 12, class: "atlas-glow" }));
    const told = sun.notes.length === 0 ? "no status" : `status of ${day(sun.updated)}`;
    const body = this.body(layout, layout.sun, "atlas-sun");
    body.firstChild.textContent = `${layout.name} · ${told} · last decision ${day(sun.last_decision)}`;
    group.append(body);
    if (sun.stale) {
      group.append(node("circle", { cx: x, cy: y, r: 9, class: "atlas-stale-ring" }));
      group.append(node("text", { x, y: y + below, class: "atlas-warning", "text-anchor": "middle" }, `stale: ${told} · decision ${day(sun.last_decision)}`));
    }
    return group;
  }

  planet(layout, body) {
    const { planet, at, radius } = body;
    const count = planet.notes.length;
    const classes = ["atlas-planet", planet.name, planet.repeated > 0 ? "repeated" : "", count === 0 ? "empty" : ""].filter(Boolean).join(" ");
    const group = node("g");
    if (planet.name === "decisions" && count >= 50) {
      group.append(node("ellipse", { cx: at.x, cy: at.y, rx: radius + 5, ry: (radius + 5) * 0.35, class: "atlas-ring" }));
    }
    const circle = this.body(layout, body, classes);
    if (planet.repeated > 0) circle.firstChild.textContent += ` · ${planet.repeated} given again`;
    group.append(circle);
    if (planet.repeated > 0) {
      group.append(node("text", { x: at.x + radius + 3, y: at.y + 3, class: "atlas-warning atlas-again" }, `${planet.repeated} again`));
    }
    return group;
  }

  // Notes the INDEX does not name: small rocks drifting past the last orbit.
  asteroids(lost, place, distance) {
    const random = seeded(lost.length * 7919);
    const group = node("g", { class: "atlas-asteroids" });
    lost.forEach((note, index) => {
      const at = polar(place, distance, distance, 150 + index * 11 + random() * 4);
      const size = 2.4 + random() * 1.2;
      const points = [0, 1, 2, 3, 4].map((corner) => {
        const angle = (corner / 5) * Math.PI * 2;
        const reach = size * (0.7 + random() * 0.6);
        return `${(at.x + reach * Math.cos(angle)).toFixed(1)},${(at.y + reach * Math.sin(angle)).toFixed(1)}`;
      });
      group.append(titled(node("polygon", { points: points.join(" ") }), `not in INDEX.md: ${note.path}`));
    });
    const label = polar(place, distance + 10, distance + 10, 150);
    group.append(node("text", { x: label.x, y: label.y, class: "atlas-small", "text-anchor": "end" }, `${lost.length} not in INDEX`));
    return group;
  }

  onSystemClick(event, layout) {
    if (this.level !== "galaxy") return;
    event.stopPropagation();
    this.showSystem(layout);
  }

  // Zooms on a system and writes what its planets are, and what its status says.
  showSystem(layout) {
    this.focus = { system: layout, planet: null };
    this.setLevel("system");
    const width = (layout.reach + 70) * 2 * 1.6;
    this.zoomTo(viewAround({ x: layout.place.x, y: layout.place.y + 12 }, width));
    const unit = width / WIDTH;
    const text = (x, y, size, classes, value, anchor = "start") =>
      node("text", { x, y, class: classes, "text-anchor": anchor, "font-size": (size * unit).toFixed(2) }, value);
    const detail = node("g");
    for (const body of layout.planets) {
      const side = body.at.x >= layout.place.x ? 1 : -1;
      const x = body.at.x + side * (body.radius + 4 * unit);
      detail.append(text(x, body.at.y - 2 * unit, 13, "atlas-label", body.planet.name, side > 0 ? "start" : "end"));
      const repeated = body.planet.repeated > 0 ? ` · ${body.planet.repeated} given again` : "";
      const count = body.planet.notes.length;
      detail.append(text(x, body.at.y + 12 * unit, 10, "atlas-label-small", `${count} ${count === 1 ? "note" : "notes"}${repeated}`, side > 0 ? "start" : "end"));
    }
    if (layout.sun) {
      const sun = layout.system.sun;
      const newest = [...sun.notes].sort((one, other) => (other.date ?? "").localeCompare(one.date ?? ""))[0];
      const told = newest ? `status of ${day(sun.updated)}: ${newest.description}` : "no status";
      const clipped = told.length > 110 ? `${told.slice(0, 109)}…` : told;
      // Under the system's name and counts, where no neighbor reaches.
      detail.append(text(layout.place.x, layout.place.y + layout.reach + 48, 11, sun.stale ? "atlas-warning-label" : "atlas-label-small", clipped, "middle"));
    }
    this.detailLayer.replaceChildren(detail);
    for (const other of this.layouts) other.group.classList.toggle("focused", other === layout);
  }

  // Zooms on a planet; its notes come out around it in a spiral, the oldest closest.
  showPlanet(layout, body) {
    this.focus = { system: layout, planet: body };
    this.setLevel("planet");
    this.zoomTo(viewAround(body.at, PLANET_VIEW));
    const unit = PLANET_VIEW / WIDTH;
    const notes = [...body.planet.notes].sort((one, other) => (one.date ?? "").localeCompare(other.date ?? ""));
    const outer = 330 * unit;
    const inner = body.radius + 18 * unit;
    const step = (outer - inner) / Math.sqrt(Math.max(notes.length, 1));
    const dot = Math.min(step * 0.38, 5 * unit);
    const group = node("g");
    group.append(node("circle", { cx: body.at.x, cy: body.at.y, r: outer + 8 * unit, class: "atlas-orbit" }));
    this.noteTargets = [];
    notes.forEach((note, index) => {
      const at = polar(body.at, inner + step * Math.sqrt(index + 0.5), inner + step * Math.sqrt(index + 0.5), index * 137.508);
      const classes = ["atlas-note-dot", body.planet.name, note.lost ? "lost" : ""].filter(Boolean).join(" ");
      const circle = node("circle", { cx: at.x.toFixed(2), cy: at.y.toFixed(2), r: dot.toFixed(2), class: classes });
      const said = `${day(note.date)} · ${note.description}`;
      circle.addEventListener("mouseenter", () => this.say(said));
      circle.addEventListener("mouseleave", () => this.say());
      circle.addEventListener("click", (event) => {
        if (this.level !== "planet") return;
        event.stopPropagation();
        this.showNote(note);
      });
      group.append(titled(circle, said));
      this.noteTargets.push({ at, radius: dot + 3 * unit, label: said, open: () => this.showNote(note) });
    });
    const name = `${layout.name === "core" ? "core" : layout.name} · ${body.planet.name} · ${notes.length} notes`;
    group.append(node("text", { x: body.at.x, y: body.at.y - outer - 14 * unit, class: "atlas-label", "text-anchor": "middle", "font-size": (14 * unit).toFixed(2) }, name));
    this.notesLayer.replaceChildren(group);
  }

  async showNote(note) {
    this.setLevel("note");
    const head = html("header");
    head.append(html("h2", "", note.description), html("p", "muted", `${note.date ?? ""} · ${note.path}`));
    const body = html("pre", "", "reading…");
    this.reader.replaceChildren(head, body);
    this.reader.hidden = false;
    try {
      const text = await invoke("memory_note", { path: note.path });
      // The header is already shown above: the text starts after it.
      body.textContent = text.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, "").trim();
    } catch (error) {
      body.textContent = `${error}`;
    }
  }

  cartouche(atlas) {
    const total = atlas.systems.reduce((sum, system) => sum + notesOf(system), notesOf(atlas.center));
    const group = node("g", { class: "atlas-cartouche" });
    group.append(node("text", { x: 56, y: 76, class: "atlas-title" }, "~ $ memory atlas"));
    group.append(node("text", { x: 56, y: 96, class: "atlas-small" }, `${total} notes · ${atlas.systems.length} systems · ${new Date().toLocaleDateString()}`));
    if (atlas.warnings.length > 0) group.append(node("text", { x: 56, y: 112, class: "atlas-warning" }, `not read: ${atlas.warnings.join(" · ")}`));
    return group;
  }

  legend() {
    const group = node("g", { class: "atlas-legend", transform: `translate(${WIDTH - 300} ${HEIGHT - 128})` });
    const rows = [
      ["atlas-sun", "sun: the project's status"],
      ["atlas-sun pale", "pale sun: status older than the last decision"],
      ["atlas-planet decisions", "planets: decisions · knowledge · lessons"],
      ["atlas-planet repeated", "red: a lesson given again"],
    ];
    rows.forEach(([shape, text], index) => {
      group.append(node("circle", { cx: 6, cy: index * 18, r: 4, class: shape }));
      group.append(node("text", { x: 18, y: index * 18 + 3, class: "atlas-small" }, text));
    });
    group.append(node("polygon", { points: "3,70 8,68 10,73 5,76 2,74", class: "atlas-asteroid-key" }));
    group.append(node("text", { x: 18, y: 75, class: "atlas-small" }, "asteroid: a note INDEX.md does not name"));
    return group;
  }
}
