// The settings window (Ctrl+,). The accounts of kindric-accounts: their state, adding one,
// logging one in, taking one off the list. The memory: the size above which a session is warned
// about what it opens at startup. Rust does each change and reads
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

function plural(count, word) {
  return `${count} ${word}${count === 1 ? "" : "s"}`;
}

export class SettingsWindow {
  constructor(dialog) {
    this.dialog = dialog;
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
    this.dialog.showModal();
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
      return;
    }
    this.list.replaceChildren(...this.accounts.map((account) => this.row(account)));
  }

  row(account) {
    const item = element("li", "account");
    const state = account.state.replace("-", " ");
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
}
