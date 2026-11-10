// The settings window (Ctrl+,). The accounts of kindric-accounts: their state, adding one,
// logging one in, taking one off the list. The memory: the size above which a session is warned
// about what it opens at startup. The shortcuts: a new key for one, or its own back.
// One category shows at a time; typing in the search shows what matches in all of them. Rust does each change and reads
// the accounts back (accounts.rs); the session list sees the change by itself (refresher.rs).

import { logIn } from "./helper.js";

const { invoke } = window.__TAURI__.core;

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function button(text, onClick, className) {
  const node = element("button", className, text);
  node.type = "button";
  node.addEventListener("click", onClick);
  return node;
}

// Keys a terminal needs (Linux, Windows) or that macOS keeps (Cmd+Q, Cmd+H, Cmd+M). "ctrl" is Cmd
// on a Mac, as everywhere in the app.
const KEPT = ["ctrl+c", "ctrl+v", "ctrl+d", "ctrl+z", "ctrl+r"];
const KEPT_MAC = ["ctrl+q", "ctrl+h", "ctrl+m"];
const MODIFIER_KEYS = ["Control", "Shift", "Alt", "Meta"];

function plural(count, word) {
  return `${count} ${word}${count === 1 ? "" : "s"}`;
}

export class SettingsWindow {
  constructor(dialog, { shortcuts }) {
    this.dialog = dialog;
    this.search = dialog.querySelector(".settings-search");
    this.content = dialog.querySelector(".settings-content");
    this.sections = [...dialog.querySelectorAll(".settings-section")];
    this.empty = dialog.querySelector(".settings-empty");
    this.categoryList = dialog.querySelector(".settings-categories");
    this.categories = this.sections.map((section) => section.dataset.category);
    // The category shown without a search; the last one chosen stays for the next opening.
    this.category = this.categories[0];
    this.search.addEventListener("input", () => this.filter());
    this.search.addEventListener("keydown", (event) => this.onSearchKey(event));
    this.shortcuts = shortcuts;
    this.shortcutList = dialog.querySelector(".shortcuts-list");
    this.shortcutMessage = dialog.querySelector(".shortcuts-message");
    // Id of the shortcut waiting for its new keys, or null.
    this.capturing = null;
    // Before the app's own keys: the keys pressed are the new shortcut, not a command.
    dialog.addEventListener("keydown", (event) => this.onCaptureKey(event), true);
    // Escape stops a capture first, then empties the search, then closes.
    dialog.addEventListener("cancel", (event) => {
      if (this.capturing !== null) {
        event.preventDefault();
        this.stopCapture();
      } else if (this.search.value !== "") {
        event.preventDefault();
        this.search.value = "";
        this.filter();
      }
    });
    dialog.addEventListener("close", () => (this.capturing = null));
    this.list = dialog.querySelector(".accounts-list");
    this.message = dialog.querySelector(".accounts-message");
    this.form = dialog.querySelector(".accounts-add");
    this.name = this.form.elements.name;
    // Name of the account whose removal waits for a yes, or null.
    this.confirming = null;
    this.accounts = [];
    // A newer reading wins over one still on its way.
    this.request = 0;
    this.form.addEventListener("submit", (event) => {
      event.preventDefault();
      this.add();
    });
    this.memoryForm = dialog.querySelector(".memory-warning");
    this.memoryMessage = dialog.querySelector(".memory-message");
    this.memoryForm.addEventListener("submit", (event) => {
      event.preventDefault();
      this.saveMemoryWarning();
    });
  }

  async open() {
    this.confirming = null;
    this.message.textContent = "";
    this.name.value = "";
    this.memoryMessage.textContent = "";
    this.capturing = null;
    this.shortcutMessage.textContent = "";
    this.search.value = "";
    this.drawCategories();
    this.drawShortcuts();
    this.dialog.showModal();
    this.search.focus();
    await Promise.all([this.reload(), this.loadMemoryWarning()]);
  }

  async loadMemoryWarning() {
    try {
      this.memoryForm.elements.kb.value = await invoke("memory_warning");
    } catch (error) {
      this.memoryMessage.textContent = `${error}`;
    }
  }

  // Counts from the next session: one started already keeps what it opened.
  async saveMemoryWarning() {
    const text = this.memoryForm.elements.kb.value.trim();
    if (!/^[0-9]+$/.test(text)) {
      this.memoryMessage.textContent = "a number of KB from 1 to 1024.";
      return;
    }
    const kb = Number(text);
    try {
      await invoke("set_memory_warning", { kb });
      this.memoryMessage.textContent = "saved: counts from the next session.";
    } catch (error) {
      this.memoryMessage.textContent = `${error}`;
    }
  }

  async reload() {
    const request = ++this.request;
    if (this.accounts.length === 0) this.list.replaceChildren(element("li", "muted", "reading the accounts…"));
    try {
      const accounts = await invoke("account_statuses");
      if (request !== this.request) return;
      this.accounts = accounts;
    } catch (error) {
      if (request !== this.request) return;
      this.message.textContent = `${error}`;
      this.accounts = [];
    }
    this.draw();
  }

  draw() {
    if (this.accounts.length === 0) {
      this.list.replaceChildren(element("li", "muted", "no account yet: add one below."));
    } else {
      this.list.replaceChildren(...this.accounts.map((account) => this.row(account)));
    }
    this.filter();
  }

  drawCategories() {
    this.categoryList.replaceChildren(
      ...this.categories.map((category) => {
        const item = element("li", "", category);
        item.addEventListener("click", () => this.choose(category));
        return item;
      }),
    );
    this.filter();
  }

  choose(category) {
    this.category = category;
    this.search.value = "";
    this.filter();
    this.search.focus();
  }

  // Up and Down go through the categories while the search is empty.
  onSearchKey(event) {
    if (this.search.value !== "" || (event.key !== "ArrowDown" && event.key !== "ArrowUp")) return;
    event.preventDefault();
    const step = event.key === "ArrowDown" ? 1 : -1;
    const index = this.categories.indexOf(this.category);
    this.category = this.categories[(index + step + this.categories.length) % this.categories.length];
    this.filter();
  }

  // Without a search: the chosen category, whole. With one: every line whose text, or whose
  // category's name, holds it, in all the categories.
  filter() {
    const query = this.search.value.trim().toLowerCase();
    const searching = query !== "";
    this.content.classList.toggle("searching", searching);
    let found = false;
    for (const section of this.sections) {
      const category = section.dataset.category;
      const whole = !searching || category.includes(query);
      let shown = false;
      for (const line of section.querySelectorAll("[data-search]")) {
        line.hidden = !whole && !line.dataset.search.includes(query);
        shown ||= !line.hidden;
      }
      for (const group of section.querySelectorAll(".shortcuts-group")) {
        group.hidden = ![...group.querySelectorAll("[data-search]")].some((line) => !line.hidden);
      }
      section.hidden = searching ? !shown : category !== this.category;
      found ||= !section.hidden;
    }
    this.empty.hidden = found;
    [...this.categoryList.children].forEach((item, index) => {
      item.classList.toggle("chosen", !searching && this.categories[index] === this.category);
    });
    this.categoryList.classList.toggle("bypassed", searching);
  }

  row(account) {
    const item = element("li", "account");
    const state = account.state.replace("-", " ");
    item.dataset.search = `${account.name} ${state} ${account.plan ?? ""} account`.toLowerCase();
    item.append(element("span", "account-name", account.name), element("span", `account-state ${account.state}`, state));
    const details = [account.plan, account.sessions > 0 && plural(account.sessions, "session"), account.isolated && "isolated"].filter(Boolean);
    item.append(element("span", "account-details muted", details.join(" · ")));
    const actions = element("span", "account-actions");
    if (this.confirming === account.name) {
      const running = account.sessions > 0 ? ` ${plural(account.sessions, "session")} keep running, hidden from kindric.` : "";
      item.append(element("span", "account-confirm", `take ${account.name} off the list? its folder stays: ${account.dir}.${running}`));
      actions.append(button("remove", () => this.forget(account.name), "danger"), button("cancel", () => this.confirm(null)));
    } else {
      if (account.state !== "logged-in") actions.append(button("log in", () => this.logIn(account.name)));
      actions.append(button("remove", () => this.confirm(account.name)));
    }
    item.append(actions);
    return item;
  }

  confirm(name) {
    this.confirming = name;
    this.draw();
  }

  async add() {
    const name = this.name.value.trim();
    if (name === "") return;
    this.message.textContent = "";
    let warnings;
    try {
      warnings = await invoke("add_account", { name });
    } catch (error) {
      this.message.textContent = `${error}`;
      return;
    }
    this.name.value = "";
    this.message.textContent = warnings.join(" ");
    await this.reload();
    this.logIn(name);
  }

  logIn(name) {
    // The login window goes over this one; the list is read again once it closes.
    logIn(name, () => this.reload());
  }

  async forget(name) {
    this.confirming = null;
    try {
      await invoke("forget_account", { name });
    } catch (error) {
      this.message.textContent = `${error}`;
    }
    await this.reload();
  }

  drawShortcuts() {
    const list = this.shortcuts.list();
    const groups = [...new Set(list.map((shortcut) => shortcut.group))].map((group) => {
      const block = element("div", "shortcuts-group");
      block.append(element("h3", "", group));
      for (const shortcut of list.filter((entry) => entry.group === group)) block.append(this.shortcutRow(shortcut));
      return block;
    });
    this.shortcutList.replaceChildren(...groups);
    this.filter();
  }

  shortcutRow(shortcut) {
    const row = element("div", "shortcut");
    row.dataset.search = `${shortcut.help} ${shortcut.shown} ${shortcut.group} shortcut key`.toLowerCase();
    const waiting = this.capturing === shortcut.id;
    row.append(element("span", "shortcut-help", shortcut.help), element("kbd", waiting ? "waiting" : "", waiting ? "press keys…" : shortcut.shown));
    const actions = element("span", "shortcut-actions");
    if (!shortcut.fixed && !waiting) actions.append(button("change", () => this.startCapture(shortcut.id)));
    if (shortcut.changed && !waiting) actions.append(button("reset", () => this.saveKey(shortcut.id, null)));
    row.append(actions);
    return row;
  }

  startCapture(id) {
    this.capturing = id;
    this.shortcutMessage.textContent = "press the new keys, or Escape to cancel.";
    this.drawShortcuts();
  }

  stopCapture() {
    this.capturing = null;
    this.shortcutMessage.textContent = "";
    this.drawShortcuts();
  }

  onCaptureKey(event) {
    if (this.capturing === null) return;
    event.preventDefault();
    event.stopPropagation();
    if (MODIFIER_KEYS.includes(event.key)) return;
    if (event.key === "Escape") {
      this.stopCapture();
      return;
    }
    const name = this.shortcuts.nameOf(event);
    const refused = this.refusal(this.capturing, name);
    if (refused !== null) {
      this.shortcutMessage.textContent = refused;
      return;
    }
    this.saveKey(this.capturing, name);
  }

  // Why `name` cannot be the keys of the shortcut `id`, or null.
  refusal(id, name) {
    const { mac, shown } = this.shortcuts;
    if (name === null) return mac ? "Ctrl stays with the terminal: use Cmd." : "Super stays with the system.";
    const parts = name.split("+");
    const alone = !parts.includes("ctrl") && !parts.includes("alt");
    if (alone && !/^f([1-9]|1[0-2])$/.test(parts.at(-1))) return `${shown(name)}: add ${mac ? "Cmd" : "Ctrl"} or Alt, a key alone goes to the terminal.`;
    if ((mac ? KEPT_MAC : KEPT).includes(name)) return `${shown(name)} stays with ${mac ? "macOS" : "the terminal"}.`;
    // Alt+1 and Ctrl+Shift+1 belong to the numbered shortcuts.
    const named = /\+[1-9]$/.test(name) ? name.replace(/[1-9]$/, "#") : name;
    const other = this.shortcuts.list().find((shortcut) => shortcut.id !== id && shortcut.keys === named);
    if (other) return `${shown(name)} is already "${other.help}".`;
    return null;
  }

  // `keys` null, or the shortcut's own keys, gives it its own keys back.
  async saveKey(id, keys) {
    const shortcut = this.shortcuts.list().find((entry) => entry.id === id);
    const saved = keys === null || shortcut.isDefault(keys) ? null : keys;
    try {
      await this.shortcuts.set(id, saved);
      this.capturing = null;
      this.shortcutMessage.textContent = "";
    } catch (error) {
      this.capturing = null;
      this.shortcutMessage.textContent = `${error}`;
    }
    this.drawShortcuts();
  }
}
