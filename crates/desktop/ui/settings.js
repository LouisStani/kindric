// The settings window (Ctrl+,). The accounts of kindric-accounts: their state, adding one,
// logging one in, taking one off the list, and whether an isolated one shows in the statistics
// (stats.js). The memory: the size above which a session is warned about what it opens at startup. The sessions: purging those idle for more than 31 or 7 days.
// The shortcuts: a new key for one, or its own back.
// One category shows at a time; typing in the search shows what matches in all of them. Rust does each change and reads
// the accounts back (accounts.rs); the session list sees the change by itself (refresher.rs).

import { expiresWithin, inHowLong, timeLeft } from "./expiry.js";
import { logIn } from "./helper.js";
import { purgeQuestion, purgeResult, refusalLine } from "./purge.js";
import { statsBox } from "./stats.js";

// A login that expires within this many days can be renewed from its row.
const RENEW_DAYS = 7;

// What the form asks for each provider an account's model can come from (kindric-accounts'
// provider.rs): the address shown when none is typed (null: no address field), an example model,
// whether a key is needed, whether the model is picked among those installed. `claude` is a Claude
// login: only the name. Ollama runs on this computer at its usual address: its model is picked
// in its own list (decision 418).
const PROVIDERS = {
  claude: null,
  api: { url: null, model: "optional, e.g. claude-sonnet-4-5", key: true },
  openrouter: { url: "https://openrouter.ai/api", model: "e.g. qwen/qwen3-coder", key: true },
  ollama: { url: null, model: "e.g. qwen3.5:9b", key: false, installed: true },
  gateway: { url: "http://host:port", model: "as the gateway names it", key: true },
};

// "qwen3.5:9b · 6.6 GB"
function modelLabel(model) {
  return `${model.name} · ${(model.size / 1e9).toFixed(1)} GB`;
}

// The provider and model of an account whose model is not a Claude login: "openrouter qwen3";
// "codex" for an account that runs Codex, "opencode" (its provider) for one that runs OpenCode.
export function providerLabel(account) {
  if (account.agent === "codex") return "codex";
  if (!account.provider || account.provider === "claude") return null;
  return account.model ? `${account.provider} ${account.model}` : account.provider;
}

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

// The purge of old sessions (decisions 361, 362): a limit chosen, the count asked from Rust and
// shown for a yes, then the purge, which can take a while (Claude Code stops and removes them one by one).
class PurgeSection {
  constructor(node, onTabs) {
    this.onTabs = onTabs;
    this.choices = node.querySelector(".purge-choices");
    this.confirmLine = node.querySelector(".purge-confirm");
    this.question = node.querySelector(".purge-question");
    this.message = node.querySelector(".purge-message");
    this.refused = node.querySelector(".purge-refused");
    // The limit waiting for a yes, or null.
    this.days = null;
    for (const choice of node.querySelectorAll("[data-days]")) {
      choice.addEventListener("click", () => this.ask(Number(choice.dataset.days)));
    }
    node.querySelector(".purge-go").addEventListener("click", () => this.run());
    node.querySelector(".purge-cancel").addEventListener("click", () => this.reset());
  }

  reset() {
    this.days = null;
    this.confirmLine.hidden = true;
    this.choices.hidden = false;
    this.message.textContent = "";
    this.refused.replaceChildren();
  }

  async ask(days) {
    this.reset();
    let counts;
    try {
      counts = await invoke("purge_preview", { days });
    } catch (error) {
      this.message.textContent = `${error}`;
      return;
    }
    const question = purgeQuestion(counts, days);
    if (question === null) {
      this.message.textContent = `no session idle for more than ${days} days.`;
      return;
    }
    this.days = days;
    this.question.textContent = question;
    this.choices.hidden = true;
    this.confirmLine.hidden = false;
    this.confirmLine.querySelector(".purge-go").focus();
  }

  async run() {
    const days = this.days;
    if (days === null) return;
    this.reset();
    this.choices.hidden = true;
    this.message.textContent = "purging…";
    let purged;
    try {
      purged = await invoke("purge_sessions", { days });
    } catch (error) {
      this.reset();
      this.message.textContent = `${error}`;
      return;
    }
    this.choices.hidden = false;
    this.message.textContent = purgeResult(purged.report);
    this.refused.replaceChildren(...purged.report.refused.map((refused) => element("li", "", refusalLine(refused))));
    this.onTabs(purged.tabs);
  }
}

export class SettingsWindow {
  // `onAccounts(accounts)`: the accounts each time they are read, for the rest of the app.
  // `onAtlas()`: opens the memory atlas, once the settings are closed.
  // `onTabs(tabs)`: the tabs after a purge, whose panes may have emptied.
  constructor(dialog, { shortcuts, onAccounts, onAtlas, onTabs }) {
    this.dialog = dialog;
    this.onAccounts = onAccounts;
    this.onTabs = onTabs;
    this.purge = new PurgeSection(dialog.querySelector(".purge"), (tabs) => this.onTabs?.(tabs));
    dialog.querySelector(".memory-atlas-open").addEventListener("click", () => {
      dialog.close();
      onAtlas();
    });
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
    this.form.elements.kind.addEventListener("change", () => this.showProviderFields());
    // A newer reading of Ollama's models wins over one still on its way.
    this.modelsRequest = 0;
    // Name of the account whose removal waits for a yes, or null.
    this.confirming = null;
    // Name of the provider account whose key is being typed in its row, or null.
    this.keying = null;
    this.accounts = [];
    // The accounts whose isolated sessions show in the statistics (`stats_isolated` of the settings).
    this.statsIsolated = [];
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
    this.keying = null;
    this.message.textContent = "";
    this.name.value = "";
    this.form.elements.kind.value = "claude";
    for (const field of ["url", "model", "key"]) this.form.elements[field].value = "";
    this.showProviderFields();
    this.memoryMessage.textContent = "";
    this.purge.reset();
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
      const [accounts, settings] = await Promise.all([invoke("account_statuses"), invoke("settings")]);
      if (request !== this.request) return;
      this.accounts = accounts;
      this.statsIsolated = settings.stats_isolated ?? [];
      this.onAccounts?.(accounts);
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
    // Its login runs out soon: kindric renews it with the same window as "log in".
    const expiring = expiresWithin(account, RENEW_DAYS);
    const box = statsBox(account, this.statsIsolated);
    const searched = `${account.name} ${state} ${account.plan ?? ""} ${account.provider ?? ""} ${account.model ?? ""} account${expiring ? " login expires renew" : ""}${box ? " show in stats" : ""}`;
    item.dataset.search = searched.toLowerCase();
    item.append(element("span", "account-name", account.name), element("span", `account-state ${account.state}`, state));
    const provider = providerLabel(account);
    const details = [provider ?? account.plan, account.sessions > 0 && plural(account.sessions, "session"), account.isolated && "isolated"].filter(Boolean);
    const detailsNode = element("span", "account-details muted", details.join(" · "));
    if (expiring) {
      const expiry = `${details.length > 0 ? " · " : ""}login expires ${inHowLong(timeLeft(account))}`;
      detailsNode.append(element("span", "account-expiry", expiry));
    }
    item.append(detailsNode);
    const actions = element("span", "account-actions");
    if (box) actions.append(this.statsCheckbox(account.name, box.shown));
    if (this.confirming === account.name) {
      const running = account.sessions > 0 ? ` ${plural(account.sessions, "session")} keep running, hidden from kindric.` : "";
      item.append(element("span", "account-confirm", `take ${account.name} off the list? its folder stays: ${account.dir}.${running}`));
      actions.append(button("remove", () => this.forget(account.name), "danger"), button("cancel", () => this.confirm(null)));
    } else if (this.keying === account.name) {
      item.append(this.keyForm(account));
    } else if (provider && (account.agent ?? "claude-code") === "claude-code") {
      if (PROVIDERS[account.provider]?.key) {
        actions.append(button(account.state === "logged-in" ? "change key" : "set key", () => this.askKey(account.name)));
      }
      actions.append(button("remove", () => this.confirm(account.name)));
    } else {
      if (account.state !== "logged-in") actions.append(button("log in", () => this.logIn(account.name)));
      else if (expiring) actions.append(button("renew", () => this.logIn(account.name)));
      actions.append(button("remove", () => this.confirm(account.name)));
    }
    item.append(actions);
    return item;
  }

  statsCheckbox(name, shown) {
    const label = element("label", "account-stats", "show in stats");
    const input = document.createElement("input");
    input.type = "checkbox";
    input.checked = shown;
    input.addEventListener("change", () => this.setStats(name, input));
    label.prepend(input);
    return label;
  }

  // The box goes back as it was when Rust refuses the change, with the reason next to the accounts.
  async setStats(name, input) {
    const shown = input.checked;
    this.message.textContent = "";
    try {
      await invoke("set_stats_isolated", { account: name, shown });
      const rest = this.statsIsolated.filter((other) => other !== name);
      this.statsIsolated = shown ? [...rest, name] : rest;
    } catch (error) {
      input.checked = !shown;
      this.message.textContent = `${error}`;
    }
  }

  confirm(name) {
    this.confirming = name;
    this.draw();
  }

  // The address, model and key fields of the provider picked; none for a Claude login.
  showProviderFields() {
    const shape = PROVIDERS[this.form.elements.kind.value] ?? null;
    const fields = this.form.elements;
    this.form.querySelector(".provider-url").hidden = !shape || shape.url === null;
    this.form.querySelector(".provider-model").hidden = !shape;
    this.form.querySelector(".provider-key").hidden = !shape?.key;
    fields.url.placeholder = shape?.url ?? "";
    fields.model.placeholder = shape?.model ?? "";
    this.showInstalled(null);
    this.message.textContent = "";
    // A reading still on its way no longer counts.
    this.modelsRequest += 1;
    if (shape?.installed) this.loadInstalled();
  }

  // The list of installed models in place of the model field, or the field back when `models` is
  // null or empty.
  showInstalled(models) {
    const fields = this.form.elements;
    const choosing = Array.isArray(models) && models.length > 0;
    fields.installed.replaceChildren(...(choosing ? models : []).map((model) => {
      const option = element("option", "", modelLabel(model));
      option.value = model.name;
      return option;
    }));
    fields.installed.hidden = !choosing;
    fields.model.hidden = choosing;
    this.form.querySelector(".provider-model").classList.toggle("choosing", choosing);
  }

  async loadInstalled() {
    const request = ++this.modelsRequest;
    let models;
    try {
      models = await invoke("ollama_models");
    } catch (error) {
      if (request === this.modelsRequest) this.message.textContent = `${error}`;
      return;
    }
    if (request !== this.modelsRequest || this.form.elements.kind.value !== "ollama") return;
    this.message.textContent = models.length === 0 ? "Ollama has no model yet: get one with `ollama pull <name>`, or type its name." : "";
    this.showInstalled(models);
  }

  // A Claude or Codex account opens its login window; a provider account is ready once it has its
  // key.
  async add() {
    const name = this.name.value.trim();
    if (name === "") return;
    this.message.textContent = "";
    const fields = this.form.elements;
    const kind = fields.kind.value;
    const shape = PROVIDERS[kind] ?? null;
    const request = { name };
    // Codex is not a provider of Claude Code's model but another program: its own login only.
    if (kind === "codex") request.agent = "codex";
    if (shape) {
      const model = fields.installed.hidden ? fields.model.value.trim() : fields.installed.value;
      const url = shape.url === null ? null : fields.url.value.trim() || null;
      request.provider = { kind, url, model: model || null };
      request.key = shape.key ? fields.key.value : null;
    }
    let warnings;
    try {
      warnings = await invoke("add_account", request);
    } catch (error) {
      this.message.textContent = `${error}`;
      return;
    } finally {
      fields.key.value = "";
    }
    this.name.value = "";
    fields.url.value = "";
    fields.model.value = "";
    this.message.textContent = warnings.join(" ");
    await this.reload();
    if (!shape) this.logIn(name);
  }

  askKey(name) {
    this.keying = name;
    this.draw();
    this.list.querySelector(".account-key input")?.focus();
  }

  // The row's field for a provider account's key: kindric hands it to kindric-accounts and keeps
  // nothing of it.
  keyForm(account) {
    const form = element("form", "account-key");
    const input = element("input");
    input.type = "password";
    input.spellcheck = false;
    input.autocomplete = "off";
    input.placeholder = `${account.provider} key for ${account.name}`;
    const save = element("button", "", "save");
    save.type = "submit";
    form.append(input, save, button("cancel", () => this.askKey(null)));
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const key = input.value;
      input.value = "";
      try {
        await invoke("set_account_key", { name: account.name, key });
        this.keying = null;
        this.message.textContent = "";
      } catch (error) {
        this.message.textContent = `${error}`;
      }
      await this.reload();
    });
    return form;
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
    const keys = element("kbd", waiting ? "waiting" : "", waiting ? "press keys…" : shortcut.shown);
    // Focusable, to hold the focus while it waits for the keys (startCapture).
    if (waiting) keys.tabIndex = -1;
    row.append(element("span", "shortcut-help", shortcut.help), keys);
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
    // The "change" button pressed is gone with the rows drawn again, and the focus with it, out
    // of the dialog, whose listener would then never get the keys.
    this.shortcutList.querySelector("kbd.waiting")?.focus({ preventScroll: true });
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
