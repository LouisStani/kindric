// The usage window (F3): the week of each Claude account and where its pace leads, the tokens of
// each project and session, and the cache written hour by hour. Rust reads the transcripts and the
// history of the status lines (`stats`); usage-board.js works out what to show.

import { PERIODS, byProject, bySession, cacheColumns, newTokens, tokensText, total, weekOf, weekText } from "./usage-board.js";

const { invoke } = window.__TAURI__.core;

const HINT = "← → period · esc close";

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

// "Nov 26 04:38", in the user's time and words.
function dateText(seconds) {
  return new Date(seconds * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

function dayText(seconds) {
  return new Date(seconds * 1000).toLocaleDateString([], { month: "short", day: "numeric" });
}

// Sizes are set through the CSSOM: the page's policy refuses style attributes.
function bar(className, share) {
  const track = element("span", `bar ${className}`);
  const fill = element("span", "bar-fill");
  fill.style.setProperty("width", `${Math.max(0, Math.min(1, share)) * 100}%`);
  track.append(fill);
  return track;
}

function section(title, note) {
  const box = element("section", "usage-section");
  const heading = element("h2", "", title);
  if (note) heading.append(element("span", "usage-section-note", note));
  box.append(heading);
  return box;
}

export class UsageWindow {
  // `labelOf(account, sessionId)`: the name of a session kindric lists, or null.
  constructor(dialog, { labelOf }) {
    this.dialog = dialog;
    this.labelOf = labelOf;
    this.periods = dialog.querySelector(".usage-periods");
    this.body = dialog.querySelector(".usage-body");
    dialog.querySelector(".usage-hint").textContent = HINT;
    this.days = PERIODS[0];
    this.data = null;
    this.request = 0;
    dialog.addEventListener("keydown", (event) => this.onKey(event));
  }

  async open() {
    this.dialog.showModal();
    this.dialog.focus({ preventScroll: true });
    if (this.data) this.render();
    else this.body.replaceChildren(element("p", "muted", "reading the transcripts…"));
    this.renderPeriods();
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
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    event.stopPropagation();
    const step = event.key === "ArrowRight" ? 1 : -1;
    this.show(PERIODS[Math.min(Math.max(PERIODS.indexOf(this.days) + step, 0), PERIODS.length - 1)]);
  }

  show(days) {
    this.days = days;
    this.renderPeriods();
    if (this.data) this.render();
  }

  renderPeriods() {
    this.periods.replaceChildren(
      ...PERIODS.map((days) => {
        const choice = element("button", days === this.days ? "on" : "", `${days} days`);
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

  render() {
    const now = Math.floor(Date.now() / 1000);
    const { summary, history, problems } = this.data;
    this.body.replaceChildren(
      ...this.weeks(history, now),
      this.tokens("tokens by project", byProject(summary, this.days, now), total(summary, this.days, now)),
      this.tokens("tokens by session", bySession(summary, this.days, now, this.labelOf), null),
      this.cache(summary, now),
      ...problems.map((problem) => element("p", "usage-problem", problem)),
    );
  }

  weeks(history, now) {
    const box = section("weekly limit", "Claude subscriptions");
    for (const [name, figures] of Object.entries(history)) {
      const week = weekOf(figures, now);
      if (!week) continue;
      const percent = week.kind === "over" ? null : week.percent;
      const hot = week.kind === "full" || week.kind === "full-at";
      const line = element("div", "usage-week");
      line.append(
        element("strong", "", name),
        bar(hot ? "hot" : "", (percent ?? 0) / 100),
        element("span", "usage-figure", percent === null ? "–" : `${Math.round(percent)}%`),
        element("span", "usage-week-text", weekText(week, dateText)),
      );
      if (week.old && week.kind !== "over") line.append(element("span", "usage-week-age", `figures from ${dateText(week.readAt)}`));
      box.append(line);
    }
    return box.querySelector(".usage-week") ? [box] : [];
  }

  // A line per row: the bar counts the new tokens, the cache read again has its own column.
  tokens(title, rows, sum) {
    const note = sum ? `${tokensText(newTokens(sum))} new, ${tokensText(sum.cache_read)} read again from the cache` : "since each one started";
    const box = section(title, note);
    if (rows.length === 0) {
      box.append(element("p", "muted", "nothing in this period"));
      return box;
    }
    const head = element("div", "usage-row usage-row-head");
    head.append(element("span"), element("span"), element("span", "usage-figure", "new"), element("span", "usage-figure", "cache read"));
    box.append(head);
    const most = Math.max(1, ...rows.filter((row) => !row.rest).map((row) => newTokens(row.tokens)));
    for (const row of rows) {
      const { input, output, cache_creation: written } = row.tokens;
      const line = element("div", row.rest ? "usage-row is-rest" : "usage-row");
      const name = element("span", "usage-name", row.name);
      name.title = row.title;
      if (row.project) name.append(element("span", "usage-project", ` · ${row.project}`));
      const track = row.rest ? element("span") : bar("", newTokens(row.tokens) / most);
      track.title = `input ${tokensText(input)} · output ${tokensText(output)} · cache written ${tokensText(written)}`;
      line.append(name, track, element("span", "usage-figure", tokensText(newTokens(row.tokens))), element("span", "usage-figure", tokensText(row.tokens.cache_read)));
      box.append(line);
    }
    return box;
  }

  cache(summary, now) {
    const columns = cacheColumns(summary, this.days, now);
    const sum = columns.reduce((all, column) => all + column.tokens, 0);
    const said = `${tokensText(sum)}, ${this.days === 7 ? "by hour" : "by day"}`;
    const box = section("cache written", said);
    const most = Math.max(...columns.map((column) => column.tokens));
    if (most === 0) {
      box.append(element("p", "muted", "nothing in this period"));
      return box;
    }
    const byHour = this.days === 7;
    const chart = element("div", byHour ? "usage-columns" : "usage-columns is-days");
    chart.setAttribute("role", "img");
    chart.setAttribute("aria-label", `cache written over ${this.days} days: ${said}`);
    // The column under the mouse, in words; the busiest one otherwise.
    const top = columns.find((column) => column.tokens === most);
    const caption = element("p", "usage-caption");
    const tell = (column, what) => {
      caption.textContent = `${what}${byHour ? dateText(column.at) : dayText(column.at)} · ${tokensText(column.tokens)}`;
    };
    for (const column of columns) {
      const slot = element("span", "usage-column");
      const fill = element("span", "usage-column-fill");
      fill.style.setProperty("height", `${(column.tokens / most) * 100}%`);
      slot.append(fill);
      slot.onmouseenter = () => tell(column, "");
      chart.append(slot);
    }
    chart.onmouseleave = () => tell(top, "most: ");
    tell(top, "most: ");
    box.append(chart, caption);
    return box;
  }
}
