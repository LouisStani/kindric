// The overview (decisions 324 to 331, page B): number 0 in the bar, the page the user lives on.
// A summary of where the sessions stand, then a column per project with a card per session.
// A card opens its session.

import { STANDINGS, columnsOf, lineOf, standing, summaryOf } from "./overview-board.js";

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

// Narrowest a column gets before the last one gathers the others.
const COLUMN_WIDTH = 300;

export class Overview {
  // `onOpen(row)` opens the session of a card.
  constructor(section, { onOpen }) {
    this.section = section;
    this.onOpen = onOpen;
    this.summary = element("header", "overview-summary");
    this.board = element("div", "overview-board");
    section.append(this.summary, this.board);
  }

  // `rows` are the sessions the bar would show, `tabs` `[{ id, name }]` in the order of the bar.
  render(view, rows, tabs, newSessionKey) {
    this.renderSummary(rows);
    if (!view.loaded) {
      this.board.replaceChildren(element("p", "overview-note", "loading…"));
      return;
    }
    if (rows.length === 0) {
      this.board.replaceChildren(element("p", "overview-note", `no sessions yet · ${newSessionKey} starts one`));
      return;
    }
    const most = Math.floor(this.section.clientWidth / COLUMN_WIDTH);
    this.board.replaceChildren(...columnsOf(rows, tabs, most).map((column) => this.column(column)));
  }

  renderSummary(rows) {
    const counts = summaryOf(rows);
    this.summary.replaceChildren(
      ...STANDINGS.map((group) => {
        const item = element("span", `overview-count is-${group.id}`);
        item.append(element("span", "dot"), element("strong", "", `${counts[group.id]}`), element("span", "", group.label));
        return item;
      }),
      element("span", "overview-hint", "click or ⏎ on a card opens it"),
    );
  }

  column(column) {
    const section = element("section", "overview-column");
    const head = element("h2", "", column.names.join(" · "));
    head.append(element("span", "count", `${column.cards.length}`));
    section.append(head, ...column.cards.map((card) => this.card(card)));
    return section;
  }

  card({ row, project }) {
    const card = element("article", `overview-card is-${standing(row)}`);
    const head = element("div", "card-head");
    head.append(element("span", "dot"), element("span", "card-name", row.label));
    if (row.unread > 0) head.append(element("span", "card-badge", `${row.unread} msg`));
    head.append(element("span", "card-where", project ? `${row.account} · ${project}` : row.account));
    card.append(head, element("p", "card-line", lineOf(row)));
    card.title = `${row.title}\n${row.directory}`;
    // A session shown in a terminal outside kindric cannot be opened here.
    if (row.short_id) {
      card.tabIndex = 0;
      card.classList.add("opens");
      card.onclick = () => this.onOpen(row);
      card.onkeydown = (event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          this.onOpen(row);
        }
      };
    }
    return card;
  }
}
