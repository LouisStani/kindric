// The memory atlas (Settings › memory, decisions 270-278): the memory drawn as a star chart. Each
// project is a system whose sun is its status and whose planets are its decisions, knowledge and
// lessons; what belongs to no project is the core. A pale sun is a status older than the last
// decision, a red planet holds a lesson given more than once, an asteroid is a note the zone's
// INDEX.md does not name. Rust reads the files (atlas.rs); this module only draws.

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

function node(tag, attributes = {}, text) {
  const element = document.createElementNS(SVG, tag);
  for (const [name, value] of Object.entries(attributes)) element.setAttribute(name, value);
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

export class AtlasWindow {
  constructor(dialog, { onError }) {
    this.dialog = dialog;
    this.svg = dialog.querySelector(".atlas-map");
    this.onError = onError;
    this.atlas = null;
  }

  async open() {
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
  }

  draw() {
    const atlas = this.atlas;
    const sky = node("g");
    sky.append(this.chart(), this.stars());
    const systems = node("g");
    const biggest = Math.max(1, ...atlas.systems.map(notesOf));
    const mostDecisions = Math.max(1, ...atlas.systems.map((system) => planetOf(system, "decisions").notes.length));
    atlas.systems.forEach((system, index) => {
      const angle = (index / atlas.systems.length) * 360 - 90;
      const place = polar(CENTER, ORBIT.x, ORBIT.y, angle);
      sky.append(node("line", { x1: CENTER.x, y1: CENTER.y, x2: place.x, y2: place.y, class: "atlas-thread" }));
      systems.append(this.system(system, place, notesOf(system) / biggest, mostDecisions));
    });
    this.svg.replaceChildren(sky, this.core(atlas.center), systems, this.cartouche(atlas), this.legend());
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
  core(center) {
    const core = node("g", { class: "atlas-core" });
    core.append(node("circle", { cx: CENTER.x, cy: CENTER.y, r: 46, class: "atlas-halo" }));
    core.append(node("circle", { cx: CENTER.x, cy: CENTER.y, r: 3.5, class: "atlas-sun" }));
    center.planets.forEach((planet, index) => {
      const place = polar(CENTER, 26, 26, index * 90 - 45);
      const body = node("circle", { cx: place.x, cy: place.y, r: (2 + Math.sqrt(planet.notes.length)).toFixed(1), class: "atlas-planet core" });
      core.append(titled(body, `${planet.name} · ${planet.notes.length} notes`));
      const label = polar(CENTER, 40, 40, index * 90 - 45);
      core.append(node("text", { x: label.x, y: label.y + 3, class: "atlas-small", "text-anchor": label.x < CENTER.x ? "end" : "start" }, planet.name));
    });
    core.append(node("text", { x: CENTER.x, y: CENTER.y + 66, class: "atlas-name", "text-anchor": "middle" }, "CORE"));
    return core;
  }

  system(system, place, size, mostDecisions) {
    const group = node("g", { class: "atlas-system", transform: `translate(${place.x.toFixed(1)} ${place.y.toFixed(1)})` });
    const scale = 0.9 + size * 0.6;
    for (const { distance } of Object.values(PLANETS)) {
      group.append(node("circle", { r: (distance * scale).toFixed(1), class: "atlas-orbit" }));
    }
    group.append(this.sun(system.sun, system.name, 46 * scale + 46));
    for (const [name, { angle, distance }] of Object.entries(PLANETS)) {
      group.append(this.planet(planetOf(system, name), polar({ x: 0, y: 0 }, distance * scale, distance * scale, angle), mostDecisions));
    }
    const lost = lostOf(system);
    if (lost.length > 0) group.append(this.asteroids(lost, 46 * scale + 10));
    const counts = ["decisions", "knowledge", "lessons"].map((name) => planetOf(system, name).notes.length);
    group.append(node("text", { y: 46 * scale + 22, class: "atlas-name", "text-anchor": "middle" }, system.name.toUpperCase()));
    group.append(node("text", { y: 46 * scale + 34, class: "atlas-small", "text-anchor": "middle" }, `${counts[0]} d · ${counts[1]} k · ${counts[2]} l`));
    return group;
  }

  // The status. Pale when it is older than the last decision: it no longer tells where things are.
  // The warning goes under the system's name, at `below`.
  sun(sun, name, below) {
    const group = node("g", { class: sun.stale ? "atlas-star stale" : "atlas-star" });
    group.append(node("circle", { r: 12, class: "atlas-glow" }));
    const told = sun.notes.length === 0 ? "no status" : `status of ${day(sun.updated)}`;
    group.append(titled(node("circle", { r: 5, class: "atlas-sun" }), `${name} · ${told} · last decision ${day(sun.last_decision)}`));
    if (sun.stale) {
      group.append(node("circle", { r: 9, class: "atlas-stale-ring" }));
      group.append(node("text", { y: below, class: "atlas-warning", "text-anchor": "middle" }, `stale: ${told} · decision ${day(sun.last_decision)}`));
    }
    return group;
  }

  planet(planet, place, mostDecisions) {
    const count = planet.notes.length;
    const radius = planet.name === "decisions" ? 2 + 7 * Math.sqrt(count / mostDecisions) : 2 + Math.min(count, 16) ** 0.5;
    const classes = ["atlas-planet", planet.name, planet.repeated > 0 ? "repeated" : "", count === 0 ? "empty" : ""].filter(Boolean).join(" ");
    const group = node("g");
    if (planet.name === "decisions" && count >= 50) {
      group.append(node("ellipse", { cx: place.x, cy: place.y, rx: radius + 5, ry: (radius + 5) * 0.35, class: "atlas-ring" }));
    }
    const repeated = planet.repeated > 0 ? ` · ${planet.repeated} given again` : "";
    group.append(titled(node("circle", { cx: place.x.toFixed(1), cy: place.y.toFixed(1), r: radius.toFixed(1), class: classes }), `${planet.name} · ${count}${repeated}`));
    if (planet.repeated > 0) {
      group.append(node("text", { x: place.x + radius + 3, y: place.y + 3, class: "atlas-warning" }, `${planet.repeated} again`));
    }
    return group;
  }

  // Notes the INDEX does not name: small rocks drifting past the last orbit.
  asteroids(lost, distance) {
    const random = seeded(lost.length * 7919);
    const group = node("g", { class: "atlas-asteroids" });
    lost.forEach((note, index) => {
      const place = polar({ x: 0, y: 0 }, distance, distance, 150 + index * 11 + random() * 4);
      const size = 2.4 + random() * 1.2;
      const points = [0, 1, 2, 3, 4].map((corner) => {
        const angle = (corner / 5) * Math.PI * 2;
        const reach = size * (0.7 + random() * 0.6);
        return `${(place.x + reach * Math.cos(angle)).toFixed(1)},${(place.y + reach * Math.sin(angle)).toFixed(1)}`;
      });
      group.append(titled(node("polygon", { points: points.join(" ") }), `not in INDEX.md: ${note.path}`));
    });
    const last = polar({ x: 0, y: 0 }, distance + 10, distance + 10, 150);
    group.append(node("text", { x: last.x, y: last.y, class: "atlas-small", "text-anchor": "end" }, `${lost.length} not in INDEX`));
    return group;
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
    group.append(node("text", { x: 0, y: 100, class: "atlas-hint" }, "esc close"));
    return group;
  }
}
