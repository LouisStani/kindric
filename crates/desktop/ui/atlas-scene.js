// The memory atlas without the screen: where every project, system and planet is, how the
// camera sees a point, and the sky behind. Pure functions, so `node --test` can check them
// (atlas.test.js); atlas.js draws what they give.

export const TAU = Math.PI * 2;
// Planets are drawn this much bigger than their orbits need, so they can be seen and clicked.
export const PLANET_SCALE = 1.6;
// Room left between two projects, and between a project and the core.
export const PROJECT_GAP = 50;
export const START = { yaw: 0.6, pitch: 0.55 };
export const TYPES = ["gas", "rock", "ice", "lava", "ocean"];
export const FORMS = ["spiral3", "barred", "ring", "spiral2", "elliptical", "irregular", "spiral4"];
const CORE_KINDS = ["global", "setup", "perso", "skills"];

// ----- Small pure helpers -----

// The same sky and the same places at every opening.
export function seeded(seed) {
  let state = seed | 0;
  return () => {
    state = (state + 0x6d2b79f5) | 0;
    let value = Math.imul(state ^ (state >>> 15), 1 | state);
    value ^= value + Math.imul(value ^ (value >>> 7), 61 | value);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

// FNV-1a: a file keeps its look from one opening to the next.
export function hashOf(text) {
  let hash = 0x811c9dc5;
  for (let index = 0; index < text.length; index += 1) hash = Math.imul(hash ^ text.charCodeAt(index), 0x01000193);
  return hash >>> 0;
}

function gauss(random) {
  return Math.sqrt(-2 * Math.log(random() + 1e-9)) * Math.cos(TAU * random());
}

export function ease(t) {
  return t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2;
}

// `2026-11-06` → `06/11`.
export function day(date) {
  return date ? `${date.slice(8, 10)}/${date.slice(5, 7)}` : "none";
}

export function newest(notes) {
  return [...notes].sort((one, other) => (other.date ?? "").localeCompare(one.date ?? ""))[0];
}

export function hslToRgb(hue, saturation, lightness) {
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
export function tables(stops) {
  const colors = stops.map(([hue, saturation, lightness]) => hslToRgb(hue, saturation, lightness));
  return [0, 1, 2].map((channel) => colors.map((color) => color[channel].toFixed(3)).join(" "));
}

// The surfaces of the five kinds of planet (the models drawn on Claude Design, decision 291).
export const MODELS = {
  gas: { noise: ["fractalNoise", "0.0025 0.05", 4, 7], stops: (h) => [[h + 20, 40, 22], [h, 70, 46], [h + 15, 85, 72], [h - 12, 65, 38], [h + 30, 55, 86], [h + 5, 70, 58]] },
  rock: { noise: ["turbulence", "0.018", 5, 3], stops: (h) => [[h, 12, 10], [h + 10, 18, 26], [h, 22, 40], [h + 25, 20, 55], [h, 12, 72]] },
  ice: { noise: ["fractalNoise", "0.01 0.025", 4, 11], stops: (h) => [[h, 45, 40], [h + 10, 45, 62], [h, 35, 80], [0, 0, 96], [h - 10, 55, 72], [h, 40, 55]] },
  lava: { noise: ["turbulence", "0.022", 4, 5], stops: (h) => [[h, 20, 3], [h, 22, 7], [h, 25, 11], [h, 25, 13], [18, 100, 48], [42, 100, 68]] },
  ocean: { noise: ["fractalNoise", "0.011", 5, 21], stops: (h) => [[h, 85, 12], [h, 80, 22], [h, 75, 32], [h - 8, 70, 40], [115, 40, 30], [85, 35, 42], [40, 30, 60]] },
};

// Where the k-th of n systems of a galaxy sits, so that its real systems draw its form.
export function placeInForm(form, k, n, random) {
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
export function tilt(project, v) {
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
export function orbitPoint(system, radius, angle) {
  const lx = Math.cos(angle) * radius;
  const lz = Math.sin(angle) * radius;
  const y = -lz * Math.sin(system.inc);
  const z = lz * Math.cos(system.inc);
  const cn = Math.cos(system.node);
  const sn = Math.sin(system.node);
  return { x: system.center.x + lx * cn - z * sn, y: system.center.y + y, z: system.center.z + lx * sn + z * cn };
}

export function planetPosition(planet, clock) {
  return orbitPoint(planet.system, planet.orbit, planet.theta0 + planet.omega * clock);
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

export function buildScene(atlas) {
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

export function buildSky(random) {
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

export function cloudSvg(hue, seed) {
  const [r, g, b] = hslToRgb(hue, 90, 58);
  const [r2, g2, b2] = hslToRgb(hue + 40, 85, 45);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512"><defs>
<filter id="n" x="0" y="0" width="100%" height="100%"><feTurbulence type="fractalNoise" baseFrequency="0.006" numOctaves="6" seed="${seed}"/><feColorMatrix type="matrix" values="0 0 0 0 ${r}  0 0 0 0 ${g}  0 0 0 0 ${b}  2.2 0 0 0 -0.85"/></filter>
<filter id="m" x="0" y="0" width="100%" height="100%"><feTurbulence type="turbulence" baseFrequency="0.012" numOctaves="5" seed="${seed + 9}"/><feColorMatrix type="matrix" values="0 0 0 0 ${r2}  0 0 0 0 ${g2}  0 0 0 0 ${b2}  0 2.4 0 0 -0.9"/></filter>
<radialGradient id="fade"><stop offset="0.2" stop-color="#fff"/><stop offset="1" stop-color="#000"/></radialGradient>
<mask id="k"><rect width="512" height="512" fill="url(#fade)"/></mask></defs>
<g mask="url(#k)"><rect width="512" height="512" filter="url(#n)"/><rect width="512" height="512" filter="url(#m)" opacity="0.7"/></g></svg>`;
}

export function textureSvg(type, hue) {
  const model = MODELS[type];
  const [kind, frequency, octaves, seed] = model.noise;
  const [r, g, b] = tables(model.stops(hue));
  const clouds = type === "ocean"
    ? `<filter id="c" x="0" y="0" width="100%" height="100%"><feTurbulence type="fractalNoise" baseFrequency="0.008 0.03" numOctaves="4" seed="4" stitchTiles="stitch"/><feColorMatrix type="matrix" values="0 0 0 0 1  0 0 0 0 1  0 0 0 0 1  0 0 0 2.6 -1.35"/></filter>`
    : "";
  const cloudLayer = type === "ocean" ? `<rect width="400" height="300" filter="url(#c)" opacity="0.8"/>` : "";
  return `<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300"><defs><filter id="f" x="0" y="0" width="100%" height="100%"><feTurbulence type="${kind}" baseFrequency="${frequency}" numOctaves="${octaves}" seed="${seed}" stitchTiles="stitch"/><feColorMatrix type="saturate" values="0"/><feComponentTransfer><feFuncR type="table" tableValues="${r}"/><feFuncG type="table" tableValues="${g}"/><feFuncB type="table" tableValues="${b}"/><feFuncA type="table" tableValues="1 1"/></feComponentTransfer></filter>${clouds}</defs><rect width="400" height="300" filter="url(#f)"/>${cloudLayer}</svg>`;
}

export function lavaGlowSvg() {
  const [kind, frequency, octaves, seed] = MODELS.lava.noise;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300"><defs><filter id="g" x="0" y="0" width="100%" height="100%"><feTurbulence type="${kind}" baseFrequency="${frequency}" numOctaves="${octaves}" seed="${seed}" stitchTiles="stitch"/><feColorMatrix type="matrix" values="0 0 0 0 1  0 0 0 0 0.45  0 0 0 0 0.1  0 0 0 -9 2.4"/><feGaussianBlur stdDeviation="1.2"/></filter></defs><rect width="400" height="300" filter="url(#g)"/></svg>`;
}



// ----- The camera: a target, a distance, a yaw and a pitch; the screen: its width, height and lens -----

export function anglesOf(camera) {
  return { cy: Math.cos(camera.yaw), sy: Math.sin(camera.yaw), cp: Math.cos(camera.pitch), sp: Math.sin(camera.pitch) };
}

// Where `point` falls on the screen, with its depth; null behind the camera.
export function projectPoint(camera, angles, screen, point) {
  const { target, dist } = camera;
  const { cy, sy, cp, sp } = angles;
  const rx = point.x - target.x;
  const ry = point.y - target.y;
  const rz = point.z - target.z;
  const x1 = rx * cy - rz * sy;
  const z1 = rx * sy + rz * cy;
  const y2 = ry * cp + z1 * sp;
  const z2 = -ry * sp + z1 * cp + dist;
  if (z2 < 0.5) return null;
  return { x: screen.width / 2 + (screen.lens * x1) / z2, y: screen.height / 2 - (screen.lens * y2) / z2, z: z2 };
}

// Where a direction of the sky falls on the screen: it turns with the camera, never comes closer.
export function projectDirection(angles, screen, d) {
  const { cy, sy, cp, sp } = angles;
  const x1 = d.x * cy - d.z * sy;
  const z1 = d.x * sy + d.z * cy;
  const y2 = d.y * cp + z1 * sp;
  const z2 = -d.y * sp + z1 * cp;
  if (z2 < 0.05) return null;
  return { x: screen.width / 2 + (screen.lens * x1) / z2, y: screen.height / 2 - (screen.lens * y2) / z2, z: z2 };
}

// The point of the target's level plane under a screen point: zooming keeps it in place.
export function groundUnder(camera, screen, sx, sy) {
  const { yaw, pitch, dist, target } = camera;
  const dx = (sx - screen.width / 2) / screen.lens;
  const dy = -(sy - screen.height / 2) / screen.lens;
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
export function moveCenters(scene, clock) {
  const angle = clock * 0.012;
  const c = Math.cos(angle);
  const s = Math.sin(angle);
  for (const project of scene.projects) {
    if (project.isCore) continue;
    project.center = { x: (project.dir.x * c - project.dir.z * s) * project.dist, y: project.dir.y * project.dist, z: (project.dir.x * s + project.dir.z * c) * project.dist };
    for (const system of project.systems) {
      if (!system.home) {
        system.center = project.center;
        continue;
      }
      const a = system.home.angle + project.turn * clock * 0.05;
      const local = tilt(project, { x: Math.cos(a) * system.home.radius, y: system.home.height * 3 + system.home.thick, z: Math.sin(a) * system.home.radius });
      system.center = { x: project.center.x + local.x, y: project.center.y + local.y, z: project.center.z + local.z };
    }
  }
}
