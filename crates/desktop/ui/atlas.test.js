// Tests of the atlas's scene (decision 305): `node --test crates/desktop/ui/atlas.test.js`. The memory below has
// the shape atlas.rs gives: a core, small projects, galaxies of parts.

import { test } from "node:test";
import assert from "node:assert/strict";
import { buildScene, moveCenters, placeInForm, FORMS, seeded, anglesOf, projectPoint, groundUnder } from "./atlas-scene.js";

function note(path, { bytes = 1200, lost = false } = {}) {
  return { path, description: path, date: "2026-11-01", lost, bytes };
}

function part(kind, label, count, prefix, extra = {}) {
  return { kind, label, notes: Array.from({ length: count }, (_, index) => note(`${prefix}/${kind}-${index}.md`, extra)), repeated: 0 };
}

// A project of `decisions` decisions in runs of 24, as atlas.rs cuts them.
function project(name, decisions, { galaxy = decisions > 24, lessonsAgain = 0, lost = 0 } = {}) {
  const prefix = `projets/${name}`;
  const parts = [part("knowledge", "knowledge", 5, prefix), { ...part("lessons", "lessons", 1, prefix), repeated: lessonsAgain }];
  if (lost > 0) parts.push(part("others", "others", lost, prefix, { lost: true }));
  for (let start = 0; start < decisions; start += 24) {
    const count = Math.min(24, decisions - start);
    parts.push({ kind: "decisions", label: `${start + 1}–${start + count}`, notes: Array.from({ length: count }, (_, index) => note(`${prefix}/decisions/${start + index}.md`)), repeated: 0 });
  }
  return { name, sun: { notes: [note(`${prefix}/status/now.md`)], updated: "2026-11-01", last_decision: "2026-11-02", stale: true }, parts, galaxy };
}

function memory() {
  return {
    center: { name: "", sun: null, parts: [part("global", "global", 4, "global"), part("skills", "skills", 7, "skills")], galaxy: false },
    systems: [
      project("big", 290),
      project("kindric", 284, { lessonsAgain: 2 }),
      project("middle", 150, { lost: 3 }),
      project("small", 8, { galaxy: false }),
      project("tiny", 1, { galaxy: false }),
      project("other", 60),
    ],
    warnings: [],
  };
}

function distance(a, b) {
  return Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z);
}

test("no project touches another one or the core, now and later", () => {
  const scene = buildScene(memory());
  for (const clock of [0, 7.5, 120]) {
    moveCenters(scene, clock);
    const projects = scene.projects;
    for (let i = 0; i < projects.length; i += 1) {
      for (let j = i + 1; j < projects.length; j += 1) {
        const [a, b] = [projects[i], projects[j]];
        assert.ok(distance(a.center, b.center) >= a.radius + b.radius, `${a.name} and ${b.name} overlap at ${clock} s`);
      }
    }
  }
});

test("a galaxy has a system for each part and one for its status; a small project has one", () => {
  const atlas = memory();
  const scene = buildScene(atlas);
  for (const source of atlas.systems) {
    const built = scene.projects.find((each) => each.name === source.name);
    assert.equal(built.systems.length, source.galaxy ? source.parts.length + 1 : 1, source.name);
  }
});

test("every file is drawn once: a planet, or a status on its sun", () => {
  const atlas = memory();
  const scene = buildScene(atlas);
  const all = (system) => [...system.parts.flatMap((each) => each.notes), ...(system.sun?.notes ?? [])].map((each) => each.path);
  const expected = [...all(atlas.center), ...atlas.systems.flatMap(all)].sort();
  const drawn = scene.systems.flatMap((system) => [...system.planets.map((planet) => planet.note.path), ...(system.status ?? []).map((each) => each.path)]).sort();
  assert.deepEqual(drawn, expected);
});

test("a file keeps its kind of planet when the projects around it change", () => {
  const first = buildScene(memory());
  const fewer = memory();
  fewer.systems = fewer.systems.filter((each) => each.name !== "big");
  const second = buildScene(fewer);
  const types = (scene) => new Map(scene.systems.flatMap((system) => system.planets.map((planet) => [planet.note.path, planet.type])));
  const before = types(first);
  for (const [path, type] of types(second)) assert.equal(type, before.get(path), path);
});

test("a file INDEX.md does not name is a rock without rings; only a lesson given again is red", () => {
  const planets = buildScene(memory()).systems.flatMap((system) => system.planets);
  const lost = planets.filter((planet) => planet.note.lost);
  assert.equal(lost.length, 3);
  for (const planet of lost) {
    assert.equal(planet.type, "rock");
    assert.equal(planet.ring, false);
  }
  const red = planets.filter((planet) => planet.repeated).map((planet) => planet.note.path);
  assert.deepEqual(red, ["projets/kindric/lessons-0.md"]);
});

test("every form puts the status at the centre of its galaxy and the other systems away from it", () => {
  for (const form of FORMS) {
    const random = seeded(7);
    assert.equal(placeInForm(form, 0, 15, random).radius, 0, form);
    for (let k = 1; k < 15; k += 1) assert.ok(placeInForm(form, k, 15, random).radius > 0, `${form} ${k}`);
  }
});

test("the point found under the pointer is drawn under the pointer", () => {
  const screen = { width: 1400, height: 860, lens: 946 };
  const camera = { target: { x: 120, y: -40, z: 300 }, dist: 2500, yaw: 0.6, pitch: 0.55 };
  for (const [x, y] of [[700, 430], [200, 150], [1300, 800], [900, 600]]) {
    const ground = groundUnder(camera, screen, x, y);
    const seen = projectPoint(camera, anglesOf(camera), screen, ground);
    assert.ok(Math.abs(seen.x - x) < 1e-6 && Math.abs(seen.y - y) < 1e-6, `${x},${y} came back at ${seen.x},${seen.y}`);
  }
});
