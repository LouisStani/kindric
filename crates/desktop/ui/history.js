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

// When a match was written, `seconds` after 1970: the time today, else the day, with its year
// when not this one.
export function whenOf(seconds, now, locale = []) {
  if (seconds === null || seconds === undefined) return "";
  const then = new Date(seconds * 1000);
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
  // `search(query, number)` gives the rows found, `number` higher for a newer search, which makes
  // an older one stop; `sessionItem(row, when)` draws a session as Ctrl+P does; `choose(row)`
  // opens it.
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
    // Counts the searches: an answer to one that is not the last asked is dropped.
    this.asked = 0;
    this.timer = null;
    // The rows shown are not those of the words in the field: typing goes on.
    this.stale = false;
    this.query.addEventListener("input", () => this.later());
    this.query.addEventListener("keydown", (event) => this.onKey(event));
    dialog.addEventListener("close", () => this.forget());
  }

  // `where` says what is searched: "this tab" or "every session".
  open(where) {
    this.forget();
    this.query.value = "";
    this.query.placeholder = `search what was said, ${where}`;
    this.rows = [];
    this.selected = 0;
    this.say("type words: every one must be in the same message");
    this.dialog.showModal();
    this.query.focus();
  }

  // No search waits or runs any more: what it would answer is dropped.
  forget() {
    clearTimeout(this.timer);
    this.timer = null;
    this.asked += 1;
  }

  later() {
    this.forget();
    this.stale = true;
    this.timer = setTimeout(() => this.run(), PAUSE);
  }

  // Searches the words in the field; true once its rows are shown, false when a newer search or
  // the window's closing made it pointless.
  async run() {
    clearTimeout(this.timer);
    this.timer = null;
    const query = this.query.value;
    const asked = ++this.asked;
    if (wordsOf(query).length === 0) {
      this.rows = [];
      this.stale = false;
      this.say("type words: every one must be in the same message");
      return true;
    }
    this.hint.textContent = "searching…";
    let rows;
    try {
      rows = await this.search(query, asked);
    } catch (error) {
      if (asked === this.asked) this.say(`${error}`);
      return false;
    }
    if (asked !== this.asked) return false;
    this.rows = rows;
    this.selected = 0;
    this.stale = false;
    this.render(wordsOf(query));
    return true;
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

  async onKey(event) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (this.stale) return;
      const step = event.key === "ArrowDown" ? 1 : -1;
      this.selected = Math.min(Math.max(this.selected + step, 0), Math.max(this.rows.length - 1, 0));
      this.render(wordsOf(this.query.value));
    } else if (event.key === "Enter") {
      event.preventDefault();
      // Rows of words no longer in the field are not opened: the search of these first.
      if (this.stale && !(await this.run())) return;
      const row = this.rows[this.selected];
      if (row) this.pick(row);
    }
  }
}
