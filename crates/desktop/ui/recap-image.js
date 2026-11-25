// Draws the recap (recap-board.js) on a canvas, to be posted: always in the colors of kindric
// (ember), whatever the theme. Blur is drawn with shadows: WebKit has no canvas filters.

import { THEMES } from "./themes.js";

export const WIDTH = 1080;
export const HEIGHT = 1350;

const MARGIN = 64;
const INNER = WIDTH - 2 * MARGIN;
const FONT = '"JetBrains Mono"';
const COLORS = { ...THEMES.ember, night: "#150a04", flame: "#f2801f", coal: "#d9731f", glow: "#ffd27a", empty: "#26160b" };
// From a cold ember to a white one.
const HEAT = [[74, 44, 20], [190, 104, 30], [247, 186, 68], [255, 248, 236]];
const SPARKS = 150;

const BARS = { top: 444, height: 96 };
const MAP_TOP = 556;
const NUMBERS = { columns: [MARGIN, 316, 554, 792], rows: [1000, 1086, 1172] };
const BAND = { top: 1254, height: 62 };
// The width the big number may take, left of the pace.
const HERO_WIDTH = 690;

function heat(strength) {
  const at = Math.min(strength, 0.9999) * (HEAT.length - 1);
  const step = Math.floor(at);
  const mixed = HEAT[step].map((from, channel) => Math.round(from + (HEAT[step + 1][channel] - from) * (at - step)));
  return `rgb(${mixed.join(" ")})`;
}

// The same sparks in every image.
function randoms(seed) {
  let state = seed;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 2 ** 32;
  };
}

function dayText(seconds) {
  return new Date(seconds * 1000).toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

class Painter {
  constructor(context) {
    this.context = context;
  }

  text(words, x, y, { size, color, bold = false, align = "left" }) {
    this.context.font = `${bold ? 700 : 400} ${size}px ${FONT}`;
    this.context.textAlign = align;
    this.context.fillStyle = color;
    this.context.fillText(words, x, y);
  }

  // What `draw` draws, over a glow of `color`.
  glowing(color, blur, draw) {
    this.context.save();
    this.context.shadowColor = color;
    this.context.shadowBlur = blur;
    draw();
    this.context.restore();
  }

  box(x, y, width, height, radius, color) {
    this.context.fillStyle = color;
    this.context.beginPath();
    this.context.roundRect(x, y, width, height, radius);
    this.context.fill();
  }

  // A soft light around a point.
  light(x, y, radiusX, radiusY, color, strength) {
    const { context } = this;
    context.save();
    context.translate(x, y);
    context.scale(1, radiusY / radiusX);
    const fill = context.createRadialGradient(0, 0, 0, 0, 0, radiusX);
    fill.addColorStop(0, color);
    fill.addColorStop(1, "transparent");
    context.globalAlpha = strength;
    context.fillStyle = fill;
    context.fillRect(-radiusX, -radiusX, 2 * radiusX, 2 * radiusX);
    context.restore();
  }

  gradient(top, bottom, stops) {
    const fill = this.context.createLinearGradient(0, top, 0, bottom);
    stops.forEach(([at, color]) => fill.addColorStop(at, color));
    return fill;
  }

  background(recap) {
    this.context.fillStyle = COLORS.night;
    this.context.fillRect(0, 0, WIDTH, HEIGHT);
    this.light(400, 300, 760, 400, COLORS.coal, 0.4);
    this.light(WIDTH / 2, 1330, 640, 180, COLORS.coal, 0.3);
    this.text("~ $", MARGIN, 78, { size: 30, bold: true, color: COLORS.waiting });
    this.text("kindric", MARGIN + 72, 78, { size: 30, bold: true, color: COLORS.text });
    this.text(`${dayText(recap.from)} to ${dayText(recap.to)}`, WIDTH - MARGIN, 78, { size: 26, align: "right", color: COLORS.muted });
  }

  sparks() {
    const random = randoms(7);
    const { context } = this;
    context.save();
    for (let spark = 0; spark < SPARKS; spark += 1) {
      context.globalAlpha = 0.15 + random() * 0.7;
      context.fillStyle = random() > 0.5 ? COLORS.waiting : COLORS.glow;
      context.beginPath();
      context.arc(MARGIN - 30 + random() * (INNER + 60), 100 + random() * 225, 0.6 + random() * 2.4, 0, 2 * Math.PI);
      context.fill();
    }
    context.restore();
  }

  // The big number, letter by letter: closer to each other than the font puts them.
  hero(words) {
    const { context } = this;
    let size = 290;
    const advance = () => size * 0.53;
    while (advance() * words.length > HERO_WIDTH) size -= 10;
    const letters = (color) => [...words].forEach((letter, at) => this.text(letter, MARGIN - 14 + at * advance(), 330, { size, bold: true, color }));
    this.glowing(COLORS.flame, 110, () => letters(COLORS.flame));
    this.glowing(COLORS.glow, 26, () => letters(COLORS.glow));
    letters(this.gradient(330 - size * 0.73, 340, [[0, "#ffffff"], [0.38, COLORS.glow], [0.62, COLORS.waiting], [1, "#c8651a"]]));
    context.textAlign = "left";
  }

  pace(recap) {
    this.text(recap.perMinute, WIDTH - MARGIN, 240, { size: 74, bold: true, align: "right", color: COLORS["text-strong"] });
    this.text("every minute,", WIDTH - MARGIN, 276, { size: 21, align: "right", color: COLORS.muted });
    this.text("day and night", WIDTH - MARGIN, 304, { size: 21, align: "right", color: COLORS.muted });
  }

  // A column per day, the best one white, its words on the side with room.
  bars(recap, column) {
    const most = Math.max(1, ...recap.perDay);
    const fill = this.gradient(BARS.top, BARS.top + BARS.height, [[0, "#f7c35c"], [1, "#8a4413"]]);
    recap.perDay.forEach((tokens, day) => {
      if (tokens === 0) return;
      const height = Math.max(3, (tokens / most) * BARS.height);
      const best = day === recap.best?.day;
      this.box(MARGIN + day * column + 2, BARS.top + BARS.height - height, column - 4, height, 3, best ? COLORS["text-strong"] : fill);
    });
    if (!recap.best) return;
    const words = `${recap.best.tokens} on the best day`;
    const left = recap.best.day >= recap.perDay.length / 2;
    const x = MARGIN + (recap.best.day + (left ? 0 : 1)) * column + (left ? -8 : 8);
    this.context.font = `400 21px ${FONT}`;
    const width = this.context.measureText(words).width;
    // Over the bars it crosses, its own ground keeps it readable.
    this.box(left ? x - width - 6 : x - 6, BARS.top - 6, width + 12, 28, 5, "rgb(26 14 6 / 80%)");
    this.text(words, x, BARS.top + 15, { size: 21, align: left ? "right" : "left", color: COLORS["text-strong"] });
  }

  // A column per day, a square per two hours.
  map(recap, column) {
    const squares = recap.squares.flatMap((day, x) => day.map((tokens, y) => ({ tokens, x: MARGIN + x * column + 1.5, y: MAP_TOP + y * column + 1.5 })));
    const side = column - 3;
    for (const square of squares) this.box(square.x, square.y, side, side, 5, COLORS.empty);
    const lit = squares.filter((square) => square.tokens > 0).map((square) => ({ ...square, strength: Math.sqrt(square.tokens / recap.most) }));
    for (const square of lit.filter((each) => each.strength > 0.4)) {
      const color = heat(square.strength);
      this.glowing(color, 34 * square.strength, () => this.box(square.x, square.y, side, side, 5, color));
    }
    for (const square of lit) this.box(square.x, square.y, side, side, 5, heat(square.strength));
  }

  numbers(recap) {
    recap.numbers.forEach(({ value, label, hot }, at) => {
      const x = NUMBERS.columns[at % NUMBERS.columns.length];
      const y = NUMBERS.rows[Math.floor(at / NUMBERS.columns.length)];
      const draw = () => this.text(value, x, y, { size: 46, bold: true, color: hot ? "#f7c35c" : COLORS["text-strong"] });
      if (hot) this.glowing(COLORS.waiting, 20, draw);
      else draw();
      this.text(label, x, y + 28, { size: 21, color: COLORS.muted });
    });
  }

  band(words) {
    const fill = this.context.createLinearGradient(MARGIN, 0, WIDTH - MARGIN, 0);
    fill.addColorStop(0, "#f7c35c");
    fill.addColorStop(1, COLORS.coal);
    this.glowing(COLORS.flame, 26, () => this.box(MARGIN, BAND.top, INNER, BAND.height, 10, fill));
    this.text(words, WIDTH / 2, BAND.top + 41, { size: 28, bold: true, align: "center", color: COLORS["on-accent"] });
  }
}

// The fonts must be there before the canvas writes with them.
export async function drawRecap(canvas, recap) {
  await Promise.all([document.fonts.load(`400 21px ${FONT}`), document.fonts.load(`700 46px ${FONT}`)]);
  canvas.width = WIDTH;
  canvas.height = HEIGHT;
  const painter = new Painter(canvas.getContext("2d"));
  const column = INNER / recap.perDay.length;
  painter.background(recap);
  painter.sparks();
  painter.hero(recap.tokens);
  painter.pace(recap);
  painter.text(`tokens in ${recap.perDay.length} days`, MARGIN, 386, { size: 42, bold: true, color: COLORS.text });
  painter.text(`${recap.fresh} new, the rest read again from the cache`, MARGIN, 418, { size: 21, color: COLORS.muted });
  painter.bars(recap, column);
  painter.map(recap, column);
  painter.numbers(recap);
  if (recap.novels) painter.band(recap.novels);
}

// The bytes of the image on `canvas`.
export async function pngOf(canvas) {
  const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) throw new Error("the image could not be made");
  return new Uint8Array(await blob.arrayBuffer());
}
