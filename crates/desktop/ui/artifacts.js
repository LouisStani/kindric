// The artifacts window (Ctrl+Shift+A): the files sessions kept for the user in the project of the
// visible tab, unsorted first then one block per group. Rust owns the files (artifacts.rs): every
// change is a command, and this module draws what it reads back.

import { imageType } from "./panes.js";

const { invoke } = window.__TAURI__.core;

const HINT = "a add · enter open · m move · n new group · r rename the group · del delete · esc close";
// A press that moves less than this is a click, not a drag.
const DRAG_PIXELS = 5;

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

// A row is a group's head ({ type: "group", group }) or a file ({ type: "file", group, name, … }).
function sameRow(one, other) {
  return one !== null && other !== null && one.type === other.type && one.group === other.group && one.name === other.name;
}

function when(seconds) {
  if (!seconds) return "";
  return new Date(seconds * 1000).toLocaleString([], { dateStyle: "short", timeStyle: "short" });
}

export class ArtifactsWindow {
  constructor(dialog, { onError }) {
    this.dialog = dialog;
    this.onError = onError;
    this.title = dialog.querySelector(".artifacts-title");
    this.body = dialog.querySelector(".artifacts-body");
    this.hint = dialog.querySelector(".artifacts-hint");
    this.project = null;
    this.blocks = [];
    // Every row in the order shown: each group's head, then its files.
    this.rows = [];
    this.selected = 0;
    // "list", or a step over it: "view", "move", "name", "confirm".
    this.mode = "list";
    this.choices = [];
    this.choice = 0;
    this.naming = null;
    this.thumbnails = new Map();
    this.request = 0;
    // True while the system's file picker is open: a second press does not open another one.
    this.adding = false;
    dialog.querySelector(".artifacts-add").onclick = () => this.add();
    dialog.addEventListener("keydown", (event) => this.onKey(event));
    // Escape closes the window only from the list: a step over it goes back to the list first.
    dialog.addEventListener("cancel", (event) => {
      if (this.mode === "list") return;
      event.preventDefault();
      this.back();
    });
    dialog.addEventListener("close", () => this.forgetThumbnails());
  }

  // `project` is the folder of the visible tab's project, null for "other".
  async open(project, label) {
    this.project = project;
    this.selected = 0;
    this.mode = "list";
    this.title.textContent = `artifacts · ${label}`;
    this.body.replaceChildren();
    this.dialog.showModal();
    await this.reload(null);
  }

  args(extra) {
    return { project: this.project, ...extra };
  }

  current() {
    return this.rows[this.selected] ?? null;
  }

  // The file chosen, or null on a group's head.
  currentFile() {
    const row = this.current();
    return row && row.type === "file" ? row : null;
  }

  // Reads the files again; `keep` is the file to select afterwards, if it is still there.
  async reload(keep) {
    const request = ++this.request;
    let blocks;
    try {
      blocks = await invoke("artifacts_view", this.args({}));
    } catch (error) {
      if (request === this.request) this.body.replaceChildren(element("p", "muted", `${error}`));
      return;
    }
    if (request !== this.request) return;
    this.blocks = blocks;
    this.rows = blocks.flatMap((block) => [
      { type: "group", group: block.name, name: null, count: block.files.length },
      ...block.files.map((file) => ({ ...file, type: "file", group: block.name })),
    ]);
    const found = keep === null ? -1 : this.rows.findIndex((row) => sameRow(row, keep));
    const firstFile = this.rows.findIndex((row) => row.type === "file");
    const fallback = this.selected > 0 ? this.selected : Math.max(firstFile, 0);
    this.selected = found >= 0 ? found : Math.min(fallback, Math.max(this.rows.length - 1, 0));
    this.mode = "list";
    this.render();
  }

  render() {
    this.hint.textContent = this.mode === "list" ? HINT : "esc back";
    // The keys reach the window only while it has the focus (a form takes it to type a name).
    if (this.mode !== "name") this.dialog.focus({ preventScroll: true });
    if (this.mode === "view") return this.renderView();
    if (this.mode === "move") return this.renderChoices();
    if (this.mode === "name") return this.renderNaming();
    if (this.mode === "confirm") return this.renderConfirm();
    this.renderList();
  }

  renderList() {
    if (this.rows.length <= 1) {
      this.body.replaceChildren(element("p", "muted", "nothing kept yet: sessions keep the files you ask for with keep_artifact, or press a to add one"));
      return;
    }
    const list = element("ol", "artifacts-list");
    this.rows.forEach((row, index) => list.append(row.type === "group" ? this.head(row, index) : this.item(row, index)));
    this.body.replaceChildren(list);
    list.querySelector(".selected")?.scrollIntoView({ block: "nearest" });
  }

  head(row, index) {
    const head = element("li", "artifact-group", `${row.group ?? "unsorted"} · ${row.count}`);
    head.classList.toggle("selected", index === this.selected);
    head.dataset.group = row.group ?? "";
    head.onclick = () => {
      this.selected = index;
      this.render();
    };
    return head;
  }

  item(file, index) {
    const row = element("li", "artifact");
    row.classList.toggle("selected", index === this.selected);
    const thumb = element("span", "artifact-thumb", file.image ? "" : (file.name.split(".").pop() || "file").slice(0, 4));
    if (file.image) this.thumbnail(file).then((url) => url && thumb.replaceChildren(Object.assign(element("img"), { src: url, alt: "" })));
    row.append(thumb, element("span", "artifact-name", file.name), element("span", "artifact-date", when(file.modified)));
    row.title = file.group ? `${file.group}/${file.name}` : file.name;
    row.onclick = () => {
      this.selected = index;
      this.openFile(file);
    };
    row.onmousedown = (event) => this.drag(event, row, file);
    return row;
  }

  async thumbnail(file) {
    const key = `${file.group ?? ""}/${file.name}`;
    if (!this.thumbnails.has(key)) {
      this.thumbnails.set(
        key,
        invoke("artifact_bytes", this.args({ group: file.group, name: file.name })).then(
          (bytes) => URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: imageType(file.name) })),
          () => null,
        ),
      );
    }
    return this.thumbnails.get(key);
  }

  async forgetThumbnails() {
    const urls = await Promise.all(this.thumbnails.values());
    for (const url of urls) if (url) URL.revokeObjectURL(url);
    this.thumbnails.clear();
  }

  async renderView() {
    const file = this.currentFile();
    if (!file) return;
    const url = await this.thumbnail(file);
    const view = element("div", "artifact-view");
    if (url) view.append(Object.assign(element("img"), { src: url, alt: file.name }));
    else view.append(element("p", "muted", `${file.name} cannot be shown`));
    view.append(element("p", "artifact-caption", file.group ? `${file.group}/${file.name}` : file.name));
    view.onclick = () => this.back();
    this.body.replaceChildren(view);
  }

  renderChoices() {
    const file = this.currentFile();
    const list = element("ol", "artifacts-list");
    list.append(element("li", "artifact-group", `move ${file.name} to`));
    this.choices.forEach((choice, index) => {
      const row = element("li", "artifact", choice.label);
      row.classList.toggle("selected", index === this.choice);
      row.onclick = () => {
        this.choice = index;
        this.chooseTarget();
      };
      list.append(row);
    });
    this.body.replaceChildren(list);
  }

  renderNaming() {
    const form = element("form", "artifact-naming");
    const label = element("label", "", this.naming.label);
    const input = Object.assign(element("input"), { value: this.naming.value, spellcheck: false, autocomplete: "off" });
    label.append(input);
    form.append(label);
    form.onsubmit = (event) => {
      event.preventDefault();
      this.submitName(input.value);
    };
    this.body.replaceChildren(form);
    input.focus();
    input.select();
  }

  renderConfirm() {
    const row = this.current();
    const text =
      row.type === "file"
        ? `delete ${row.name}? enter deletes it, esc keeps it`
        : `remove the group ${row.group}? its files go back to unsorted. enter removes it, esc keeps it`;
    this.body.replaceChildren(element("p", "artifact-confirm", text));
  }

  back() {
    this.mode = "list";
    this.render();
  }

  async run(command, extra, keep) {
    try {
      await invoke(command, this.args(extra));
    } catch (error) {
      this.onError(`${error}`);
    }
    await this.reload(keep);
  }

  // Files chosen in the system's picker are copied into the group of the selected row, or into
  // unsorted; the first one is selected afterwards.
  async add() {
    if (this.adding || this.mode !== "list") return;
    this.adding = true;
    const group = this.current()?.group ?? null;
    let added = { names: [], errors: [] };
    try {
      added = await invoke("add_artifacts", this.args({ group }));
    } catch (error) {
      added.errors = [`${error}`];
    } finally {
      this.adding = false;
    }
    if (added.errors.length > 0) this.onError(added.errors.join(" · "));
    const first = added.names.length > 0 ? { type: "file", group, name: added.names[0] } : this.current();
    await this.reload(first);
  }

  async openFile(file) {
    if (file.image) {
      this.mode = "view";
      this.render();
      return;
    }
    try {
      await invoke("open_artifact", this.args({ group: file.group, name: file.name }));
    } catch (error) {
      this.onError(`${error}`);
    }
  }

  startMove() {
    const file = this.currentFile();
    if (!file) return;
    const groups = this.blocks.map((block) => block.name).filter((name) => name !== file.group);
    this.choices = [...groups.map((name) => ({ label: name ?? "unsorted", group: name })), { label: "new group…", create: true }];
    this.choice = 0;
    this.mode = "move";
    this.render();
  }

  chooseTarget() {
    const target = this.choices[this.choice];
    const file = this.currentFile();
    if (!target || !file) return;
    if (target.create) {
      this.startNaming("new group for this file", "", { moveFile: file });
      return;
    }
    this.moveFile(file, target.group);
  }

  moveFile(file, to) {
    if (file.group === to) return this.back();
    return this.run("move_artifact", { group: file.group, name: file.name, to }, null);
  }

  startNaming(label, value, purpose) {
    this.naming = { label, value, ...purpose };
    this.mode = "name";
    this.render();
  }

  async submitName(text) {
    const name = text.trim();
    const naming = this.naming;
    if (!name) return this.back();
    if (naming.rename !== undefined) return this.run("rename_artifact_group", { from: naming.rename, to: name }, this.current());
    try {
      await invoke("create_artifact_group", this.args({ group: name }));
    } catch (error) {
      this.onError(`${error}`);
      return this.reload(this.current());
    }
    if (naming.moveFile) return this.run("move_artifact", { group: naming.moveFile.group, name: naming.moveFile.name, to: name }, null);
    await this.reload(this.current());
  }

  // On a group's head or one of its files: that group. "unsorted" is not a group.
  startRename() {
    const row = this.current();
    if (!row || row.group === null) {
      this.onError("choose a group, or a file of a group, to rename it");
      return;
    }
    this.startNaming(`rename the group ${row.group}`, row.group, { rename: row.group });
  }

  // A file follows the mouse and drops on a group's head; a press that does not move is a click.
  drag(event, row, file) {
    if (event.button !== 0 || this.mode !== "list") return;
    const startX = event.clientX;
    const startY = event.clientY;
    let dragging = false;
    let target = null;
    const move = (moved) => {
      if (!dragging && Math.hypot(moved.clientX - startX, moved.clientY - startY) < DRAG_PIXELS) return;
      dragging = true;
      row.classList.add("dragging");
      const head = document.elementFromPoint(moved.clientX, moved.clientY)?.closest(".artifact-group");
      if (head !== target) {
        target?.classList.remove("drop-target");
        target = head ?? null;
        target?.classList.add("drop-target");
      }
    };
    const release = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", release);
      row.classList.remove("dragging");
      target?.classList.remove("drop-target");
      if (!dragging) return;
      // The click that ends a drag does not open the file.
      row.onclick = null;
      if (target) this.moveFile(file, target.dataset.group === "" ? null : target.dataset.group);
      else this.render();
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", release);
  }

  onKey(event) {
    if (this.mode === "name") {
      if (event.key === "Escape") {
        event.preventDefault();
        this.back();
      }
      return;
    }
    if (event.key === "Escape") {
      if (this.mode === "list") return;
      event.preventDefault();
      this.back();
      return;
    }
    const handled = this.mode === "list" ? this.onListKey(event.key) : this.onStepKey(event.key);
    if (handled) {
      event.preventDefault();
      event.stopPropagation();
    }
  }

  onListKey(key) {
    const row = this.current();
    const file = this.currentFile();
    if (key === "ArrowDown" || key === "ArrowUp") {
      const step = key === "ArrowDown" ? 1 : -1;
      this.selected = Math.min(Math.max(this.selected + step, 0), Math.max(this.rows.length - 1, 0));
      this.render();
    } else if (key === "Enter" && file) this.openFile(file);
    else if (key === "a") this.add();
    else if (key === "m" && file) this.startMove();
    else if (key === "n") this.startNaming("new group", "", {});
    else if (key === "r") this.startRename();
    else if ((key === "Delete" || key === "Backspace") && row && (file || row.group !== null)) {
      this.mode = "confirm";
      this.render();
    } else return false;
    return true;
  }

  onStepKey(key) {
    if (this.mode === "move" && (key === "ArrowDown" || key === "ArrowUp")) {
      const step = key === "ArrowDown" ? 1 : -1;
      this.choice = Math.min(Math.max(this.choice + step, 0), this.choices.length - 1);
      this.render();
    } else if (this.mode === "move" && key === "Enter") this.chooseTarget();
    else if (this.mode === "confirm" && key === "Enter") {
      const row = this.current();
      if (row.type === "file") this.run("delete_artifact", { group: row.group, name: row.name }, null);
      else this.run("remove_artifact_group", { group: row.group }, null);
    } else if (this.mode === "view" && (key === "Enter" || key === " ")) this.back();
    else return false;
    return true;
  }
}
