// The overview (decisions 324 to 331, page B): number 0 in the bar, the page the user lives on.
// A summary of where the sessions stand, then a column per project with a card per session.
// A card opens its session, and shows what the session asks: answered from here (325).

import {
  STANDINGS,
  answersInOneClick,
  choicesAnswer,
  columnsOf,
  lineOf,
  permissionText,
  requestsBySession,
  standing,
  summaryOf,
} from "./overview-board.js";

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function button(className, text, onClick) {
  const node = element("button", className, text);
  node.type = "button";
  node.onclick = onClick;
  return node;
}

// Narrowest a column gets before the last one gathers the others.
const COLUMN_WIDTH = 300;

export class Overview {
  // `onOpen(row)` opens the session of a card. `onAnswer(request, answer)` answers a permission
  // or a question of AskUserQuestion; `onType(row, request, text)` types the answer to a
  // `kindric ask` into the session. Both return promises that fail with what to tell the user.
  constructor(section, { onOpen, onAnswer, onType }) {
    this.section = section;
    this.onOpen = onOpen;
    this.onAnswer = onAnswer;
    this.onType = onType;
    this.summary = element("header", "overview-summary");
    this.board = element("div", "overview-board");
    section.append(this.summary, this.board);
    // Session id → its requests, oldest first.
    this.requests = new Map();
    this.seen = "[]";
    // Kept across renders, which come several times a second while a session works: what is
    // typed (request id or "id/question" → text), what is picked (request id → question →
    // labels, or the words of "Other…"), the questions whose "Other…" is open, and what an
    // answer being sent says (request id → "sending…" or the error).
    this.drafts = new Map();
    this.chosen = new Map();
    this.others = new Set();
    this.sending = new Map();
    this.last = null;
  }

  // The requests of `overview_requests`. Returns whether they changed. What was kept for a
  // request that is gone (answered, or settled in the terminal) goes with it.
  setRequests(requests) {
    const seen = JSON.stringify(requests);
    if (seen === this.seen) return false;
    this.seen = seen;
    this.requests = requestsBySession(requests);
    const ids = new Set(requests.map((request) => request.id));
    const kept = (key) => ids.has(key.split("/")[0]);
    for (const map of [this.drafts, this.chosen, this.sending]) {
      for (const key of [...map.keys()]) if (!kept(key)) map.delete(key);
    }
    for (const key of [...this.others]) if (!kept(key)) this.others.delete(key);
    return true;
  }

  // `rows` are the sessions the bar would show, `tabs` `[{ id, name }]` in the order of the bar.
  render(view, rows, tabs, newSessionKey) {
    this.last = { view, rows, tabs, newSessionKey };
    const focused = document.activeElement instanceof HTMLElement && this.section.contains(document.activeElement) ? document.activeElement : null;
    const focusKey = focused?.dataset.key ?? null;
    const caret = focused instanceof HTMLInputElement ? focused.selectionStart : null;
    this.renderSummary(rows);
    if (!view.loaded) {
      this.board.replaceChildren(element("p", "overview-note", "loading…"));
    } else if (rows.length === 0) {
      this.board.replaceChildren(element("p", "overview-note", `no sessions yet · ${newSessionKey} starts one`));
    } else {
      const most = Math.floor(this.section.clientWidth / COLUMN_WIDTH);
      this.board.replaceChildren(...columnsOf(rows, tabs, most, this.requests).map((column) => this.column(column)));
    }
    // The field being typed in keeps the keyboard and its caret.
    const again = focusKey && this.section.querySelector(`[data-key="${CSS.escape(focusKey)}"]`);
    if (again) {
      again.focus();
      if (caret !== null && again instanceof HTMLInputElement) again.setSelectionRange(caret, caret);
    }
  }

  redraw() {
    if (this.last) {
      const { view, rows, tabs, newSessionKey } = this.last;
      this.render(view, rows, tabs, newSessionKey);
    }
  }

  renderSummary(rows) {
    const counts = summaryOf(rows, this.requests);
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
    const requests = this.requests.get(row.session_id) ?? [];
    const card = element("article", `overview-card is-${standing(row, this.requests)}`);
    const head = element("div", "card-head");
    head.append(element("span", "dot"), element("span", "card-name", row.label));
    if (requests.some((request) => request.kind === "ask")) head.append(element("span", "card-badge keeps", "keeps working"));
    if (row.unread > 0) head.append(element("span", "card-badge", `${row.unread} msg`));
    head.append(element("span", "card-where", project ? `${row.account} · ${project}` : row.account));
    card.append(head);
    // What the session asks takes the place of what it last said.
    if (requests.length === 0) card.append(element("p", "card-line", lineOf(row, this.requests)));
    for (const request of requests) card.append(this.request(row, request));
    card.title = `${row.title}\n${row.directory}`;
    // A session shown in a terminal outside kindric cannot be opened here.
    if (row.short_id) {
      card.tabIndex = 0;
      card.classList.add("opens");
      card.dataset.key = `card/${row.session_id}`;
      card.onclick = () => this.onOpen(row);
      card.onkeydown = (event) => {
        if (event.key === "Enter" && event.target === card) {
          event.preventDefault();
          this.onOpen(row);
        }
      };
    }
    return card;
  }

  request(row, request) {
    const block = element("div", `request is-${request.kind}`);
    // Answering is not opening the session.
    block.onclick = (event) => event.stopPropagation();
    if (request.kind === "permission") this.permission(block, request);
    else if (request.kind === "question") this.question(block, request);
    else this.ask(block, row, request);
    const sending = this.sending.get(request.id);
    if (sending) block.append(element("p", sending === "sending…" ? "request-note" : "request-note error", sending));
    return block;
  }

  permission(block, request) {
    const { prompt, text } = permissionText(request);
    const command = element("pre", "request-command");
    command.append(element("span", "prompt", prompt), document.createTextNode(` ${text}`));
    const actions = element("div", "request-actions");
    actions.append(
      button("primary", "Allow", () => this.answer(request, { kind: "allow" })),
      button("", "Deny", () => this.answer(request, { kind: "deny" })),
    );
    block.append(command, actions);
  }

  question(block, request) {
    const questions = Array.isArray(request.questions) ? request.questions : [];
    if (!this.chosen.has(request.id)) this.chosen.set(request.id, new Map());
    const chosen = this.chosen.get(request.id);
    const oneClick = answersInOneClick(questions);
    questions.forEach((question, index) => {
      block.append(element("p", "request-question", question.question));
      const options = element("div", "request-options");
      (question.options ?? []).forEach((option, number) => {
        const picked = chosen.get(question.question);
        const on = Array.isArray(picked) && picked.includes(option.label);
        const choice = button(on ? "option on" : "option", "", () => {
          if (oneClick) {
            this.answer(request, { kind: "choices", answers: { [question.question]: option.label } });
            return;
          }
          const before = Array.isArray(chosen.get(question.question)) ? chosen.get(question.question) : [];
          const after = question.multiSelect
            ? before.includes(option.label) ? before.filter((label) => label !== option.label) : [...before, option.label]
            : [option.label];
          chosen.set(question.question, after);
          this.others.delete(`${request.id}/${index}`);
          this.redraw();
        });
        choice.append(element("span", "option-number", `${number + 1}`), document.createTextNode(` ${option.label}`));
        if (option.description) choice.title = option.description;
        options.append(choice);
      });
      const otherKey = `${request.id}/${index}`;
      options.append(
        button(this.others.has(otherKey) ? "option other on" : "option other", "Other…", () => {
          this.others.add(otherKey);
          chosen.set(question.question, this.drafts.get(otherKey) ?? "");
          this.redraw();
          this.section.querySelector(`[data-key="${CSS.escape(otherKey)}"]`)?.focus();
        }),
      );
      block.append(options);
      if (this.others.has(otherKey)) {
        const field = this.field(otherKey, "your answer…", (text) => chosen.set(question.question, text), () => this.sendChoices(request, questions, chosen));
        block.append(field);
      }
    });
    if (!oneClick) {
      const actions = element("div", "request-actions");
      const send = button("primary", "Send", () => this.sendChoices(request, questions, chosen));
      send.disabled = choicesAnswer(questions, chosen) === null;
      actions.append(send);
      block.append(actions);
    }
  }

  sendChoices(request, questions, chosen) {
    const answers = choicesAnswer(questions, chosen);
    if (answers) this.answer(request, { kind: "choices", answers });
  }

  ask(block, row, request) {
    block.append(element("p", "request-question", request.question), element("p", "request-context", request.context));
    const send = () => {
      const text = (this.drafts.get(request.id) ?? "").trim();
      if (text) this.send(request, () => this.onType(row, request, text));
    };
    const field = this.field(request.id, "your answer…", () => {}, send);
    const actions = element("div", "request-actions answer");
    actions.append(field, button("primary", "Send", send));
    block.append(actions);
  }

  // A text field whose words survive the renders; Enter sends.
  field(key, placeholder, onText, onEnter) {
    const input = element("input", "request-field");
    input.dataset.key = key;
    input.placeholder = placeholder;
    input.spellcheck = false;
    input.value = this.drafts.get(key) ?? "";
    input.oninput = () => {
      this.drafts.set(key, input.value);
      onText(input.value);
    };
    input.onkeydown = (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        onEnter();
      }
    };
    return input;
  }

  answer(request, answer) {
    this.send(request, () => this.onAnswer(request, answer));
  }

  // "sending…" until the request goes from the list; the error if it fails.
  async send(request, sent) {
    if (this.sending.get(request.id) === "sending…") return;
    this.sending.set(request.id, "sending…");
    this.redraw();
    try {
      await sent();
    } catch (error) {
      this.sending.set(request.id, `${error}`);
      this.redraw();
    }
  }
}
