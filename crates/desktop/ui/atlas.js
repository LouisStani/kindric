// The memory atlas (Settings › memory, decisions 270-303): the whole memory in 3D, every file a
// planet. A small project is one system around its sun, its status; a big one is a galaxy of
// systems (its knowledge, its lessons, its decisions in runs, atlas.rs) whose places draw its form.
// The core holds what belongs to no project. A pale sun is a status older than the last decision,
// a red planet with a pulsing ring holds a lesson given again, a grey lumpy rock is a file
// INDEX.md does not name. Nothing is drawn that is not a file, apart from the sky behind.
// Dragging turns the view and it keeps turning a little once let go; a pinch or the wheel goes in
// toward the pointer or out, and the view settles on a state: everything, a project, a system, a
// file. Typing searches. Rust reads the files; this module only draws.

const { invoke } = window.__TAURI__.core;

const TAU = Math.PI * 2;
const SEARCH_MS = 200;
const FLIGHT_MS = 1600;
// A zoom gesture is over once no event came for this long.
const GESTURE_END_MS = 180;
// Planets are drawn this much bigger than their orbits need, so they can be seen and clicked.
const PLANET_SCALE = 1.6;
// Room left between two projects, and between a project and the core.
const PROJECT_GAP = 50;
const SPIN_MAX = 10;
const START = { yaw: 0.6, pitch: 0.55 };
const TYPES = ["gas", "rock", "ice", "lava", "ocean"];
const FORMS = ["spiral3", "barred", "ring", "spiral2", "elliptical", "irregular", "spiral4"];
const CORE_KINDS = ["global", "setup", "perso", "skills"];
const HINTS = [
  "type to search · drag or two fingers sideways to turn · pinch or wheel in and out · click a project · esc closes",
  "click a system · pinch out or esc to come back · type to search",
  "click a planet to read it · pinch out or esc to come back · type to search",
  "esc comes back",
];

// ----- Small pure helpers -----

// The same sky and the same places at every opening.
function seeded(seed) {
  let state = seed | 0;
  return () => {
    state = (state + 0x6d2b79f5) | 0;
    let value = Math.imul(state ^ (state >>> 15), 1 | state);
    value ^= value + Math.imul(value ^ (value >>> 7), 61 | value);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

// FNV-1a: a file keeps its look from one opening to the next.
function hashOf(text) {
  let hash = 0x811c9dc5;
  for (let index = 0; index < text.length; index += 1) hash = Math.imul(hash ^ text.charCodeAt(index), 0x01000193);
  return hash >>> 0;
}

function gauss(random) {
  return Math.sqrt(-2 * Math.log(random() + 1e-9)) * Math.cos(TAU * random());
}

function ease(t) {
  return t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2;
}

// `2026-11-06` → `06/11`.
function day(date) {
  return date ? `${date.slice(8, 10)}/${date.slice(5, 7)}` : "none";
}

function newest(notes) {
  return [...notes].sort((one, other) => (other.date ?? "").localeCompare(one.date ?? ""))[0];
}

function hslToRgb(hue, saturation, lightness) {
  const h = (((hue % 360) + 360) % 360) / 360;
  const s = saturation / 100;
  const l = lightness / 100;
  const channel = (n) => {
    const k = (n + h * 12) % 12;
    const a = s * Math.min(l, 1 - l);
    return l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
  };
  return [channel(0), channel(8), channel(4)];
}

// The color tables of an feComponentTransfer: noise in grey becomes the ramp of `stops`.
function tables(stops) {
  const colors = stops.map(([hue, saturation, lightness]) => hslToRgb(hue, saturation, lightness));
  return [0, 1, 2].map((channel) => colors.map((color) => color[channel].toFixed(3)).join(" "));
}

// The surfaces of the five kinds of planet (the models drawn on Claude Design, decision 291).
const MODELS = {
  gas: { noise: ["fractalNoise", "0.0025 0.05", 4, 7], stops: (h) => [[h + 20, 40, 22], [h, 70, 46], [h + 15, 85, 72], [h - 12, 65, 38], [h + 30, 55, 86], [h + 5, 70, 58]] },
  rock: { noise: ["turbulence", "0.018", 5, 3], stops: (h) => [[h, 12, 10], [h + 10, 18, 26], [h, 22, 40], [h + 25, 20, 55], [h, 12, 72]] },
  ice: { noise: ["fractalNoise", "0.01 0.025", 4, 11], stops: (h) => [[h, 45, 40], [h + 10, 45, 62], [h, 35, 80], [0, 0, 96], [h - 10, 55, 72], [h, 40, 55]] },
  lava: { noise: ["turbulence", "0.022", 4, 5], stops: (h) => [[h, 20, 3], [h, 22, 7], [h, 25, 11], [h, 25, 13], [18, 100, 48], [42, 100, 68]] },
  ocean: { noise: ["fractalNoise", "0.011", 5, 21], stops: (h) => [[h, 85, 12], [h, 80, 22], [h, 75, 32], [h - 8, 70, 40], [115, 40, 30], [85, 35, 42], [40, 30, 60]] },
};

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

// Where the k-th of n systems of a galaxy sits, so that its real systems draw its form.
function placeInForm(form, k, n, random) {
  if (k === 0) return { radius: 0, angle: 0, height: 0 };
  const i = k - 1;
  const m = Math.max(1, n - 1);
  if (form === "ring") return { radius: 230 + (random() - 0.5) * 30, angle: (i / m) * TAU, height: (random() - 0.5) * 30 };
  if (form === "elliptical") {
    const radius = 70 + Math.sqrt(i / m) * 210;
    return { radius, angle: i * 2.39996, height: (random() - 0.5) * 140 * (1 - radius / 320) };
  }
  if (form === "irregular") {
    const [angle, radius] = [[0.4, 170], [2.6, 250], [4.4, 120]][i % 3];
    return { radius: radius + (random() - 0.5) * 110, angle: angle + (random() - 0.5) * 0.9, height: (random() - 0.5) * 70 };
  }
  if (form === "barred") {
    if (i < 4) return { radius: 85 + Math.floor(i / 2) * 85, angle: i % 2 ? Math.PI : 0, height: (random() - 0.5) * 20 };
    const j = i - 4;
    const step = Math.floor(j / 2) + 1;
    return { radius: 210 + step * 32, angle: (j % 2) * Math.PI + step * 0.55, height: (random() - 0.5) * 25 };
  }
  const arms = form === "spiral2" ? 2 : form === "spiral4" ? 4 : 3;
  const step = Math.floor(i / arms) + 1;
  return { radius: 60 + step * (arms === 2 ? 52 : 70), angle: ((i % arms) * TAU) / arms + step * (arms === 2 ? 0.62 : 0.55), height: (random() - 0.5) * 25 };
}

// A galaxy lies in its own plane: tipped by `inc`, turned by `node`.
function tilt(project, v) {
  if (!project.inc) return v;
  const ci = Math.cos(project.inc);
  const si = Math.sin(project.inc);
  const cn = Math.cos(project.node);
  const sn = Math.sin(project.node);
  const y = v.y * ci - v.z * si;
  const z = v.y * si + v.z * ci;
  return { x: v.x * cn - z * sn, y, z: v.x * sn + z * cn };
}

// A point of a system's orbit plane, at `radius` and `angle` from its sun.
function orbitPoint(system, radius, angle) {
  const lx = Math.cos(angle) * radius;
  const lz = Math.sin(angle) * radius;
  const y = -lz * Math.sin(system.inc);
  const z = lz * Math.cos(system.inc);
  const cn = Math.cos(system.node);
  const sn = Math.sin(system.node);
  return { x: system.center.x + lx * cn - z * sn, y: system.center.y + y, z: system.center.z + lx * sn + z * cn };
}

function planetPosition(planet, clock) {
  return orbitPoint(planet.system, planet.orbit, planet.theta0 + planet.omega * clock);
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

// ----- The scene: projects, systems, planets, out of what atlas.rs read -----

function planetOf(note, kind, repeated, system, project, random) {
  const look = hashOf(note.path);
  const pick = seeded(look);
  const type = TYPES[look % TYPES.length];
  const scale = { gas: 1.35, rock: 0.8, ice: 1, lava: 0.9, ocean: 1.05 }[type];
  const size = Math.max(0.7, Math.min(6, (0.8 + 1.4 * Math.log10(Math.max(note.bytes, 120) / 120)) * scale));
  return {
    note,
    kind,
    repeated: kind === "lessons" && repeated > 0,
    lost: note.lost,
    system,
    project,
    type: note.lost ? "rock" : type,
    hue: (system.hue + (pick() - 0.5) * 50 + 360) % 360,
    variant: Math.floor(pick() * 3),
    ring: !note.lost && pick() < (type === "gas" ? 0.45 : 0.1),
    ringTilt: (pick() - 0.5) * 0.8,
    size,
    orbit: 0,
    theta0: random() * TAU,
    omega: 0,
    spin: (0.4 + pick() * 0.8) * (pick() < 0.5 ? -1 : 1),
    phase: pick() * 10,
  };
}

function hueOf(projectHue, kind) {
  if (kind === "knowledge") return projectHue + 55;
  if (kind === "lessons") return projectHue - 55;
  if (kind === "others") return projectHue + 180;
  if (CORE_KINDS.includes(kind)) return projectHue + CORE_KINDS.indexOf(kind) * 70;
  return projectHue;
}

// A sun and the notes around it, each on its own orbit, the first ones closest; orbits go either way.
function systemOf(project, label, hue, sunRadius, entries, random, extra = {}) {
  const star = extra.main ? "main" : ["main", "main", "giant", "dwarf", "main"][Math.floor(random() * 5)];
  const system = {
    project,
    label,
    hue: (hue + 360) % 360,
    center: { x: 0, y: 0, z: 0 },
    sunRadius: sunRadius * (star === "giant" ? 1.7 : star === "dwarf" ? 0.55 : 1),
    star,
    inc: (random() - 0.5) * 1.2,
    node: random() * TAU,
    planets: [],
    radius: 0,
    ...extra,
  };
  let orbit = system.sunRadius * 3 + 6;
  for (const { note, kind, repeated } of entries) {
    const planet = planetOf(note, kind, repeated, system, project, random);
    orbit += 2.2 + planet.size * PLANET_SCALE * 0.95 + (planet.ring ? planet.size * 1.4 : 0) + random() * 0.8;
    planet.orbit = orbit;
    planet.omega = (TAU / (7 * (orbit / 30) ** 1.5)) * (random() < 0.5 ? -1 : 1);
    system.planets.push(planet);
  }
  system.radius = orbit + 8;
  return system;
}

function buildScene(atlas) {
  const projects = [];
  const center = { name: "memory", isCore: true, hue: 42, center: { x: 0, y: 0, z: 0 }, systems: [], galaxy: false, sun: null, total: 0 };
  const coreEntries = atlas.center.parts.flatMap((part) => part.notes.map((note) => ({ note, kind: part.kind, repeated: part.repeated })));
  const core = systemOf(center, "memory", 42, 14, coreEntries, seeded(42), { main: true });
  core.status = [];
  center.systems.push(core);
  center.radius = core.radius;
  center.total = coreEntries.length;
  projects.push(center);
  let galaxies = 0;
  atlas.systems.forEach((source, index) => {
    const random = seeded(hashOf(source.name));
    // The golden angle: two projects side by side never get close colors.
    const hue = (index * 137.508 + 20) % 360;
    const status = source.sun?.notes ?? [];
    const project = {
      name: source.name,
      hue,
      sun: source.sun,
      galaxy: source.galaxy,
      systems: [],
      radius: 0,
      total: status.length + source.parts.reduce((sum, part) => sum + part.notes.length, 0),
      turn: random() < 0.5 ? -1 : 1,
      inc: 0,
      node: 0,
      center: { x: 0, y: 0, z: 0 },
    };
    const entries = (part) => part.notes.map((note) => ({ note, kind: part.kind, repeated: part.repeated }));
    if (!source.galaxy) {
      const system = systemOf(project, source.name, hue, 9, source.parts.flatMap(entries), random, { main: true, status, stale: source.sun?.stale ?? false });
      project.systems.push(system);
      project.radius = system.radius;
    } else {
      project.inc = (0.35 + random() * 0.85) * (random() < 0.5 ? -1 : 1);
      project.node = random() * TAU;
      project.form = FORMS[galaxies % FORMS.length];
      galaxies += 1;
      const parts = [{ label: "status", kind: "status", notes: [], repeated: 0 }, ...source.parts];
      parts.forEach((part, k) => {
        const place = placeInForm(project.form, k, parts.length, random);
        const extra = k === 0 ? { main: true, status, stale: source.sun?.stale ?? false } : { kind: part.kind, repeated: part.repeated };
        const system = systemOf(project, part.label, hueOf(hue, part.kind), k === 0 ? 11 : 7, entries(part), random, extra);
        system.home = { ...place, thick: (random() - 0.5) * 200 * Math.max(0.35, 1 - place.radius / 550) };
        project.systems.push(system);
        project.radius = Math.max(project.radius, place.radius + system.radius);
      });
    }
    projects.push(project);
  });
  spread(projects);
  return { projects, systems: projects.flatMap((project) => project.systems), core: center };
}

// Each project as close to the core as its size allows, then pushed apart in 3D until none touch.
function spread(projects) {
  const core = projects.find((project) => project.isCore);
  const around = projects.filter((project) => !project.isCore);
  around.forEach((project, index) => {
    project.dist = core.radius + PROJECT_GAP + project.radius;
    const y = (1 - (2 * (index + 0.5)) / around.length) * 0.55;
    const angle = index * 2.39996;
    const k = Math.sqrt(1 - y * y);
    project.dir = { x: Math.cos(angle) * k, y, z: Math.sin(angle) * k };
  });
  const at = (project) => ({ x: project.dir.x * project.dist, y: project.dir.y * project.dist, z: project.dir.z * project.dist });
  for (let round = 0; round < 600; round += 1) {
    let moved = false;
    for (let i = 0; i < around.length; i += 1) {
      for (let j = i + 1; j < around.length; j += 1) {
        const a = around[i];
        const b = around[j];
        const pa = at(a);
        const pb = at(b);
        const dx = pa.x - pb.x;
        const dy = pa.y - pb.y;
        const dz = pa.z - pb.z;
        const distance = Math.hypot(dx, dy, dz) || 1;
        const need = a.radius + b.radius + PROJECT_GAP;
        if (distance >= need) continue;
        moved = true;
        const push = ((need - distance) * 0.5) / distance;
        for (const [project, position, sign] of [[a, pa, 1], [b, pb, -1]]) {
          const q = { x: position.x + dx * push * sign, y: position.y + dy * push * sign * 0.6, z: position.z + dz * push * sign };
          const n = Math.hypot(q.x, q.y, q.z);
          // Too crowded at this distance: step outward a little.
          if (round > 200) project.dist += 2;
          const dir = { x: q.x / n, y: Math.max(-0.7, Math.min(0.7, q.y / n)), z: q.z / n };
          const m = Math.hypot(dir.x, dir.y, dir.z);
          project.dir = { x: dir.x / m, y: dir.y / m, z: dir.z / m };
        }
      }
    }
    if (!moved) break;
  }
  for (const project of around) project.center = at(project);
}

// ----- The sky behind: stars, a band of the Milky Way, a few clouds; it moves with the view only -----

function buildSky(random) {
  const randomDirection = () => {
    const u = random() * 2 - 1;
    const angle = random() * TAU;
    const s = Math.sqrt(1 - u * u);
    return { x: Math.cos(angle) * s, y: u, z: Math.sin(angle) * s };
  };
  // The band crosses the first view on a diagonal: its plane holds the view direction and a slanted line.
  const cy = Math.cos(START.yaw);
  const sy = Math.sin(START.yaw);
  const cp = Math.cos(START.pitch);
  const sp = Math.sin(START.pitch);
  const forward = { x: cp * sy, y: -sp, z: cp * cy };
  const slant = { x: cy + 0.55 * sp * sy, y: 0.55 * cp, z: -sy + 0.55 * sp * cy };
  const n = { x: forward.y * slant.z - forward.z * slant.y, y: forward.z * slant.x - forward.x * slant.z, z: forward.x * slant.y - forward.y * slant.x };
  const length = Math.hypot(n.x, n.y, n.z);
  const band = { x: n.x / length, y: n.y / length, z: n.z / length };
  const nearBand = (spread) => {
    const d = randomDirection();
    const k = d.x * band.x + d.y * band.y + d.z * band.z - gauss(random) * spread;
    const v = { x: d.x - band.x * k, y: d.y - band.y * k, z: d.z - band.z * k };
    const m = Math.hypot(v.x, v.y, v.z);
    return { x: v.x / m, y: v.y / m, z: v.z / m };
  };
  const colors = ["#fff4e6", "#e9efff", "#ffe2c4", "#cfe0ff", "#ffd6e8", "#fffbe9"];
  // Stars in a few batches of one color and one brightness: thousands of them stay cheap.
  const batches = [];
  for (const color of colors) for (const [alpha, size] of [[0.22, 0.6], [0.45, 0.8], [0.7, 1.1], [0.95, 1.6]]) batches.push({ color, alpha, size, stars: [] });
  for (let index = 0; index < 9000; index += 1) {
    const star = index % 10 < 7 ? nearBand(index % 10 < 4 ? 0.035 : 0.09) : randomDirection();
    const b = random() ** 3;
    const level = b < 0.15 ? 0 : b < 0.4 ? 1 : b < 0.75 ? 2 : 3;
    batches[Math.floor(random() * colors.length) * 4 + level].stars.push(star);
  }
  return {
    batches,
    glow: Array.from({ length: 130 }, () => ({ dir: nearBand(0.03), size: 0.07 + random() * 0.1, alpha: 0.08 + random() * 0.08, hue: 25 + random() * 30 })),
    dust: Array.from({ length: 110 }, () => ({ dir: nearBand(0.02), size: 0.02 + random() * 0.05, alpha: 0.35 + random() * 0.35 })),
    bright: Array.from({ length: 14 }, () => ({ dir: randomDirection(), hue: [210, 40, 0, 190, 280][Math.floor(random() * 5)], size: 14 + random() * 30 })),
    cloudPlaces: [200, 330, 270].map((hue, index) => ({ hue, seed: 3 + index * 17, dir: nearBand(0.35), size: 0.18 + random() * 0.14 })),
    clouds: [],
  };
}

function cloudSvg(hue, seed) {
  const [r, g, b] = hslToRgb(hue, 90, 58);
  const [r2, g2, b2] = hslToRgb(hue + 40, 85, 45);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512"><defs>
<filter id="n" x="0" y="0" width="100%" height="100%"><feTurbulence type="fractalNoise" baseFrequency="0.006" numOctaves="6" seed="${seed}"/><feColorMatrix type="matrix" values="0 0 0 0 ${r}  0 0 0 0 ${g}  0 0 0 0 ${b}  2.2 0 0 0 -0.85"/></filter>
<filter id="m" x="0" y="0" width="100%" height="100%"><feTurbulence type="turbulence" baseFrequency="0.012" numOctaves="5" seed="${seed + 9}"/><feColorMatrix type="matrix" values="0 0 0 0 ${r2}  0 0 0 0 ${g2}  0 0 0 0 ${b2}  0 2.4 0 0 -0.9"/></filter>
<radialGradient id="fade"><stop offset="0.2" stop-color="#fff"/><stop offset="1" stop-color="#000"/></radialGradient>
<mask id="k"><rect width="512" height="512" fill="url(#fade)"/></mask></defs>
<g mask="url(#k)"><rect width="512" height="512" filter="url(#n)"/><rect width="512" height="512" filter="url(#m)" opacity="0.7"/></g></svg>`;
}

function textureSvg(type, hue) {
  const model = MODELS[type];
  const [kind, frequency, octaves, seed] = model.noise;
  const [r, g, b] = tables(model.stops(hue));
  const clouds = type === "ocean"
    ? `<filter id="c" x="0" y="0" width="100%" height="100%"><feTurbulence type="fractalNoise" baseFrequency="0.008 0.03" numOctaves="4" seed="4" stitchTiles="stitch"/><feColorMatrix type="matrix" values="0 0 0 0 1  0 0 0 0 1  0 0 0 0 1  0 0 0 2.6 -1.35"/></filter>`
    : "";
  const cloudLayer = type === "ocean" ? `<rect width="400" height="300" filter="url(#c)" opacity="0.8"/>` : "";
  return `<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300"><defs><filter id="f" x="0" y="0" width="100%" height="100%"><feTurbulence type="${kind}" baseFrequency="${frequency}" numOctaves="${octaves}" seed="${seed}" stitchTiles="stitch"/><feColorMatrix type="saturate" values="0"/><feComponentTransfer><feFuncR type="table" tableValues="${r}"/><feFuncG type="table" tableValues="${g}"/><feFuncB type="table" tableValues="${b}"/><feFuncA type="table" tableValues="1 1"/></feComponentTransfer></filter>${clouds}</defs><rect width="400" height="300" filter="url(#f)"/>${cloudLayer}</svg>`;
}

function lavaGlowSvg() {
  const [kind, frequency, octaves, seed] = MODELS.lava.noise;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300"><defs><filter id="g" x="0" y="0" width="100%" height="100%"><feTurbulence type="${kind}" baseFrequency="${frequency}" numOctaves="${octaves}" seed="${seed}" stitchTiles="stitch"/><feColorMatrix type="matrix" values="0 0 0 0 1  0 0 0 0 0.45  0 0 0 0 0.1  0 0 0 -9 2.4"/><feGaussianBlur stdDeviation="1.2"/></filter></defs><rect width="400" height="300" filter="url(#g)"/></svg>`;
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
    this.cy = Math.cos(this.camera.yaw);
    this.sy = Math.sin(this.camera.yaw);
    this.cp = Math.cos(this.camera.pitch);
    this.sp = Math.sin(this.camera.pitch);
  }

  project(point) {
    const { target, dist } = this.camera;
    const rx = point.x - target.x;
    const ry = point.y - target.y;
    const rz = point.z - target.z;
    const x1 = rx * this.cy - rz * this.sy;
    const z1 = rx * this.sy + rz * this.cy;
    const y2 = ry * this.cp + z1 * this.sp;
    const z2 = -ry * this.sp + z1 * this.cp + dist;
    if (z2 < 0.5) return null;
    return { x: this.width / 2 + (this.lens * x1) / z2, y: this.height / 2 - (this.lens * y2) / z2, z: z2 };
  }

  projectDirection(d) {
    const x1 = d.x * this.cy - d.z * this.sy;
    const z1 = d.x * this.sy + d.z * this.cy;
    const y2 = d.y * this.cp + z1 * this.sp;
    const z2 = -d.y * this.sp + z1 * this.cp;
    if (z2 < 0.05) return null;
    return { x: this.width / 2 + (this.lens * x1) / z2, y: this.height / 2 - (this.lens * y2) / z2, z: z2 };
  }

  // The point of the target's level plane under the screen point: zooming keeps it in place.
  groundUnder(sx, sy) {
    const { yaw, pitch, dist, target } = this.camera;
    const dx = (sx - this.width / 2) / this.lens;
    const dy = -(sy - this.height / 2) / this.lens;
    const cy = Math.cos(yaw);
    const syaw = Math.sin(yaw);
    const cp = Math.cos(pitch);
    const sp = Math.sin(pitch);
    const down = sp - dy * cp;
    const t = down > 0.02 ? Math.min((dist * sp) / down, dist * 4) : dist;
    const x1 = t * dx;
    const y2 = t * dy;
    const zr = -dist + t;
    const ry = y2 * cp - zr * sp;
    const z1 = y2 * sp + zr * cp;
    return { x: target.x + x1 * cy + z1 * syaw, y: target.y + ry, z: target.z - x1 * syaw + z1 * cy };
  }

  // Galaxies turn as one piece, each its own way; every project turns slowly around the core.
  moveCenters() {
    const angle = this.clock * 0.012;
    const c = Math.cos(angle);
    const s = Math.sin(angle);
    for (const project of this.scene.projects) {
      if (project.isCore) continue;
      project.center = { x: (project.dir.x * c - project.dir.z * s) * project.dist, y: project.dir.y * project.dist, z: (project.dir.x * s + project.dir.z * c) * project.dist };
      for (const system of project.systems) {
        if (!system.home) {
          system.center = project.center;
          continue;
        }
        const a = system.home.angle + project.turn * this.clock * 0.05;
        const local = tilt(project, { x: Math.cos(a) * system.home.radius, y: system.home.height * 3 + system.home.thick, z: Math.sin(a) * system.home.radius });
        system.center = { x: project.center.x + local.x, y: project.center.y + local.y, z: project.center.z + local.z };
      }
    }
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
    this.moveCenters();
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
