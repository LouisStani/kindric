// The history window (Ctrl+Shift+F): the sessions whose conversation holds every word typed, in
// the visible tab, or all of them from the overview; one row per session with how many messages
// hold the words and an excerpt of the last one. Rust reads the transcripts (history.rs); Enter
// or a click opens the session as Ctrl+P does. The pure parts are checked by node --test
// (history.test.js).

// Typing goes on: the search waits for this pause after the last key.
const PAUSE = 250;
const HINT = "↑↓ move · enter open · esc close";

// The words of a query, as Rust cuts them.
export function wordsOf(query) {
  return query.split(/\s+/).filter(Boolean);
}

// `text` cut into parts, `mark` on each part that is one of `words`, in any case.
export function highlight(text, words) {
  const escaped = words.map((word) => word.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).filter(Boolean);
  if (escaped.length === 0) return [{ text, mark: false }];
  const pattern = new RegExp(`(${escaped.join("|")})`, "giu");
  return text
    .split(pattern)
    .filter((part) => part !== "")
    .map((part) => ({ text: part, mark: words.some((word) => word.toLocaleLowerCase() === part.toLocaleLowerCase()) }));
}

// When a match was written: the time today, else the day, with its year when not this one.
export function whenOf(iso, now, locale = []) {
  if (!iso) return "";
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return "";
  if (then.toDateString() === now.toDateString()) return then.toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" });
  const options = { month: "short", day: "numeric" };
  if (then.getFullYear() !== now.getFullYear()) options.year = "numeric";
  return then.toLocaleDateString(locale, options);
}

// "3 messages", "1 message".
export function countText(matches) {
  return `${matches} message${matches === 1 ? "" : "s"}`;
}

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

export class HistoryWindow {
  // `search(query)` gives the rows found; `sessionItem(row, when)` draws a session as Ctrl+P
  // does; `choose(row)` opens it.
  constructor(dialog, { search, sessionItem, choose }) {
    this.dialog = dialog;
    this.search = search;
    this.sessionItem = sessionItem;
    this.choose = choose;
    this.query = dialog.querySelector(".history-query");
    this.results = dialog.querySelector(".history-results");
    this.hint = dialog.querySelector(".history-hint");
    this.rows = [];
    this.selected = 0;
    // Answers can come back out of order: only the last asked counts.
    this.asked = 0;
    this.timer = null;
    this.query.addEventListener("input", () => this.later());
    this.query.addEventListener("keydown", (event) => this.onKey(event));
  }

  // `where` says what is searched: "this tab" or "every session".
  open(where) {
    this.query.value = "";
    this.query.placeholder = `search what was said, ${where}`;
    this.rows = [];
    this.selected = 0;
    this.say("type words: every one must be in the same message");
    this.dialog.showModal();
    this.query.focus();
  }

  later() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.run(), PAUSE);
  }

  async run() {
    const query = this.query.value;
    const asked = ++this.asked;
    if (wordsOf(query).length === 0) {
      this.rows = [];
      this.say("type words: every one must be in the same message");
      return;
    }
    this.hint.textContent = "searching…";
    let rows;
    try {
      rows = await this.search(query);
    } catch (error) {
      if (asked === this.asked) this.say(`${error}`);
      return;
    }
    if (asked !== this.asked) return;
    this.rows = rows;
    this.selected = 0;
    this.render(wordsOf(query));
  }

  say(text) {
    this.results.replaceChildren();
    this.hint.textContent = text;
  }

  render(words) {
    if (this.rows.length === 0) {
      this.say("no conversation holds every word");
      return;
    }
    const now = new Date();
    this.results.replaceChildren(
      ...this.rows.map((row, index) => {
        const item = this.sessionItem(row, whenOf(row.at, now));
        item.dataset.index = `${index}`;
        if (row.short_id) item.classList.add("openable");
        if (index === this.selected) item.classList.add("selected");
        const excerpt = element("div", "detail history-excerpt");
        for (const part of highlight(row.excerpt, words)) excerpt.append(part.mark ? element("mark", "", part.text) : document.createTextNode(part.text));
        item.append(excerpt, element("div", "detail history-count", countText(row.matches)));
        item.addEventListener("click", () => this.pick(row));
        return item;
      }),
    );
    this.hint.textContent = HINT;
    this.results.querySelector(`[data-index="${this.selected}"]`)?.scrollIntoView({ block: "nearest" });
  }

  pick(row) {
    this.dialog.close();
    this.choose(row);
  }

  onKey(event) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      this.selected = Math.min(Math.max(this.selected + step, 0), Math.max(this.rows.length - 1, 0));
      this.render(wordsOf(this.query.value));
    } else if (event.key === "Enter") {
      event.preventDefault();
      const row = this.rows[this.selected];
      if (row) this.pick(row);
      else this.run();
    }
  }
}
