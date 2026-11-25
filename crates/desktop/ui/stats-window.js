// The stats window (F4): big numbers, the tokens of each account day by day, when the work happens,
// records and habits. Rust reads the transcripts (`stats`); stats-board.js works out what to show.

import { activity, counters, durationText, novelsText, perDay, periodText, periodsOf, profile, records, strength } from "./stats-board.js";
import { newTokens, tokensText } from "./usage-board.js";

const { invoke } = window.__TAURI__.core;

const HINT = "← → period · esc close";

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function dayText(seconds) {
  return new Date(seconds * 1000).toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" });
}

function hourText(hour) {
  return new Date(2026, 0, 1, hour).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function section(title, note) {
  const box = element("section", "usage-section");
  const heading = element("h2", "", title);
  if (note) heading.append(element("span", "usage-section-note", note));
  box.append(heading);
  return box;
}

// A number with its name under it, and a word about it.
function tile(value, name, note, className = "stats-tile") {
  const box = element("div", className);
  box.append(element("strong", "stats-value", value), element("span", "stats-name", name));
  if (note) box.append(element("span", "stats-note", note));
  return box;
}

export class StatsWindow {
  // `labelOf(account, sessionId)`: the name of a session kindric lists, or null.
  constructor(dialog, { labelOf }) {
    this.dialog = dialog;
    this.labelOf = labelOf;
    this.periods = dialog.querySelector(".usage-periods");
    this.body = dialog.querySelector(".usage-body");
    dialog.querySelector(".usage-hint").textContent = HINT;
    this.days = 30;
    this.data = null;
    this.request = 0;
    dialog.addEventListener("keydown", (event) => this.onKey(event));
  }

  async open() {
    this.dialog.showModal();
    this.dialog.focus({ preventScroll: true });
    if (this.data) this.render();
    else this.body.replaceChildren(element("p", "muted", "reading the transcripts…"));
    const request = ++this.request;
    try {
      const data = await invoke("stats");
      if (request !== this.request) return;
      this.data = data;
    } catch (error) {
      if (request === this.request) this.body.replaceChildren(element("p", "error", `${error}`));
      return;
    }
    this.render();
  }

  onKey(event) {
    if ((event.key !== "ArrowLeft" && event.key !== "ArrowRight") || !this.data) return;
    event.preventDefault();
    event.stopPropagation();
    const periods = periodsOf(this.data.summary, Date.now() / 1000);
    const step = event.key === "ArrowRight" ? 1 : -1;
    this.show(periods[Math.min(Math.max(periods.indexOf(this.days) + step, 0), periods.length - 1)]);
  }

  show(days) {
    this.days = days;
    this.render();
  }

  render() {
    const now = Math.floor(Date.now() / 1000);
    const { summary, problems } = this.data;
    this.renderPeriods(periodsOf(summary, now));
    if (summary.hours.length === 0) {
      this.body.replaceChildren(element("p", "muted", "no session yet"));
      return;
    }
    this.body.replaceChildren(
      this.numbers(summary, now),
      this.accounts(summary, now),
      this.map(summary, now),
      this.bests(summary, now),
      ...problems.map((problem) => element("p", "usage-problem", problem)),
    );
  }

  renderPeriods(periods) {
    this.periods.replaceChildren(
      ...periods.map((days) => {
        const choice = element("button", days === this.days ? "on" : "", periodText(days));
        choice.type = "button";
        choice.setAttribute("aria-pressed", `${days === this.days}`);
        choice.onclick = () => {
          this.show(days);
          this.dialog.focus({ preventScroll: true });
        };
        return choice;
      }),
    );
  }

  numbers(summary, now) {
    const told = counters(summary, this.days, now);
    const box = element("section", "stats-numbers");
    const compared = told.change === null ? "" : `, ${told.change >= 0 ? "+" : ""}${told.change}% new ones`;
    const hero = tile(tokensText(told.tokens), "tokens", `${tokensText(told.fresh)} new, the rest read again from the cache${compared}`, "stats-hero");
    const grid = element("div", "stats-grid");
    grid.append(
      tile(tokensText(told.written), "tokens written", novelsText(told.novels)),
      tile(tokensText(told.thinking), "tokens of thinking"),
      tile(told.sessions.toLocaleString(), "sessions"),
      tile(told.prompts.toLocaleString(), "messages typed"),
      tile(told.replies.toLocaleString(), "replies"),
    );
    if (told.searches > 0) grid.append(tile(told.searches.toLocaleString(), "web searches"));
    box.append(hero, grid);
    return box;
  }

  // One small chart per account, all on the same scale: no color to tell accounts apart.
  accounts(summary, now) {
    const list = perDay(summary, this.days, now);
    const box = section("new tokens by day");
    const most = Math.max(1, ...list.flatMap(({ columns }) => columns.map((column) => column.tokens)));
    for (const { account, columns } of list) {
      const sum = columns.reduce((total, column) => total + column.tokens, 0);
      const line = element("div", "stats-account");
      const chart = element("div", "usage-columns is-days");
      const caption = element("span", "stats-account-sum");
      const rest = () => (caption.textContent = tokensText(sum));
      chart.setAttribute("role", "img");
      chart.setAttribute("aria-label", `${account}: ${tokensText(sum)} new tokens`);
      for (const column of columns) {
        const slot = element("span", "usage-column");
        const fill = element("span", "usage-column-fill");
        fill.style.setProperty("height", `${(column.tokens / most) * 100}%`);
        slot.append(fill);
        slot.onmouseenter = () => (caption.textContent = `${dayText(column.at)} · ${tokensText(column.tokens)}`);
        chart.append(slot);
      }
      chart.onmouseleave = rest;
      rest();
      line.append(element("strong", "usage-name", account), chart, caption);
      box.append(line);
    }
    return box;
  }

  // A column per day, a row per hour of the day.
  map(summary, now) {
    const { days, cells, most } = activity(summary, this.days, now);
    const box = section("when it happens");
    const grid = element("div", "stats-map");
    grid.style.setProperty("--days", `${days.length}`);
    grid.setAttribute("role", "img");
    grid.setAttribute("aria-label", "new tokens by day and hour of the day");
    const caption = element("p", "usage-caption");
    const rest = () => (caption.textContent = `${dayText(days[0])} to ${dayText(days.at(-1))}, midnight at the top`);
    for (let hour = 0; hour < 24; hour += 1) {
      days.forEach((day, position) => {
        const tokens = cells[position][hour];
        const cell = element("span", `stats-cell is-${strength(tokens, most)}`);
        cell.onmouseenter = () => (caption.textContent = `${dayText(day)}, ${hourText(hour)} · ${tokensText(tokens)} new tokens`);
        grid.append(cell);
      });
    }
    grid.onmouseleave = rest;
    rest();
    box.append(grid, caption);
    return box;
  }

  bests(summary, now) {
    const best = records(summary, this.days, now);
    const habits = profile(summary, this.days, now);
    const nameOf = (session) => this.labelOf(session.account, session.session) ?? session.session.slice(0, 8);
    const box = section("records and habits");
    const grid = element("div", "stats-grid");
    if (best.day) grid.append(tile(tokensText(best.day.count), "new tokens on the best day", dayText(best.day.key)));
    if (best.longest) grid.append(tile(durationText(best.longest.milliseconds), "longest run", nameOf(best.longest.session)));
    if (best.heaviest) grid.append(tile(tokensText(newTokens(best.heaviest.tokens)), "heaviest session", nameOf(best.heaviest)));
    grid.append(tile(`${best.streak}`, best.streak === 1 ? "day in a row" : "days in a row"), tile(`${best.parallel}`, "sessions at once"));
    if (habits.hour !== null) grid.append(tile(hourText(habits.hour), "busiest hour"));
    if (habits.project) grid.append(tile(habits.project, "favorite project"));
    if (habits.model) grid.append(tile(habits.model.name.replace(/^claude-/, ""), "favorite model", `${habits.model.percent}% of the replies`));
    box.append(grid);
    return box;
  }
}
