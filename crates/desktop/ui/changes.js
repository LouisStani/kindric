// The changed files window (Ctrl+Shift+G, or a click on "N files" in a pane's head): what the
// session of the active pane changed, one block per working copy, in the order the session went
// into them, then the diff of the file chosen. Rust computes both with git (changes.rs).

const { invoke } = window.__TAURI__.core;

const LIST_HINT = "↑↓ move · enter diff · esc close";
const DIFF_HINT = "↑↓ scroll · esc back";
const STARTS = {
  recorded: "since the start",
  "branch-created": "since the branch was created",
  estimated: "estimated since the start",
};
const LETTERS = { added: "A", modified: "M", deleted: "D", renamed: "R" };

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function blockHead(block) {
  if (block.type === "removed") return `${block.copy} · removed`;
  if (block.type === "failed") return `${block.copy} · ${block.error}`;
  const parts = [block.copy];
  if (block.branch) parts.push(block.branch);
  parts.push(STARTS[block.start] ?? block.start);
  return parts.join(" · ");
}

function counts(file) {
  if (file.added === null || file.removed === null) return [element("span", "change-binary", "binary")];
  return [element("span", "change-added", `+${file.added}`), element("span", "change-removed", `−${file.removed}`)];
}

function shownPath(file) {
  return file.kind.type === "renamed" ? `${file.kind.from} → ${file.path}` : file.path;
}

// The class of each line of a diff, from how it starts. Before the first `@@`, the lines are the
// header (`--- a/…`, `+++ b/…`); after it, a line starting with `-` is removed whatever follows.
function diffClasses(lines) {
  let inHunk = false;
  return lines.map((line) => {
    if (line.startsWith("@@")) {
      inHunk = true;
      return "diff-hunk";
    }
    if (line.startsWith("diff --git")) inHunk = false;
    if (!inHunk) return line.startsWith("+") && !line.startsWith("+++") ? "diff-add" : "diff-meta";
    if (line.startsWith("+")) return "diff-add";
    if (line.startsWith("-")) return "diff-del";
    return line.startsWith(" ") ? "" : "diff-meta";
  });
}

export class ChangesWindow {
  constructor(dialog) {
    this.dialog = dialog;
    this.title = dialog.querySelector(".changes-title");
    this.body = dialog.querySelector(".changes-body");
    this.hint = dialog.querySelector(".changes-hint");
    this.label = "";
    this.blocks = [];
    // Every file in the order shown, across the blocks, with its block: what ↑↓ moves over.
    this.entries = [];
    this.selected = 0;
    // "list", or "diff" over it.
    this.mode = "list";
    this.request = 0;
    dialog.addEventListener("keydown", (event) => this.onKey(event));
    // Escape closes the window from the list only: from a diff it goes back to the list.
    dialog.addEventListener("cancel", (event) => {
      if (this.mode === "list") return;
      event.preventDefault();
      this.showList();
    });
  }

  async open(account, shortId, label) {
    const request = ++this.request;
    this.label = label;
    this.mode = "list";
    this.title.textContent = `changed files · ${label}`;
    this.hint.textContent = LIST_HINT;
    this.body.replaceChildren(element("p", "muted", "counting…"));
    this.blocks = [];
    this.entries = [];
    this.selected = 0;
    this.dialog.showModal();
    this.dialog.focus({ preventScroll: true });
    let blocks;
    try {
      blocks = await invoke("session_changes", { account, shortId });
    } catch (error) {
      if (request === this.request) this.body.replaceChildren(element("p", "muted", `${error}`));
      return;
    }
    if (request !== this.request) return;
    this.blocks = blocks;
    this.entries = blocks.flatMap((block) => (block.files ?? []).map((file) => ({ block, file })));
    this.render();
  }

  showList() {
    ++this.request;
    this.mode = "list";
    this.title.textContent = `changed files · ${this.label}`;
    this.hint.textContent = LIST_HINT;
    this.dialog.focus({ preventScroll: true });
    this.render();
  }

  render() {
    if (this.entries.length === 0 && this.blocks.every((block) => block.type === "changes")) {
      this.body.replaceChildren(element("p", "muted", "no file changed since the start"));
      return;
    }
    const list = element("ol", "artifacts-list");
    let index = 0;
    for (const block of this.blocks) {
      list.append(element("li", `artifact-group${block.type === "failed" ? " change-failed" : ""}`, blockHead(block)));
      for (const file of block.files ?? []) list.append(this.item(file, index++));
    }
    this.body.replaceChildren(list);
    list.querySelector(".selected")?.scrollIntoView({ block: "nearest" });
  }

  item(file, index) {
    const row = element("li", "artifact change");
    row.classList.toggle("selected", index === this.selected);
    const path = shownPath(file);
    row.title = path;
    row.append(element("span", `change-kind kind-${file.kind.type}`, LETTERS[file.kind.type] ?? "?"), element("span", "artifact-name", path), ...counts(file));
    row.onclick = () => {
      this.selected = index;
      this.showDiff();
    };
    return row;
  }

  async showDiff() {
    const entry = this.entries[this.selected];
    if (!entry) return;
    const { block, file } = entry;
    const request = ++this.request;
    this.mode = "diff";
    this.title.textContent = `${shownPath(file)} · ${block.copy}`;
    this.hint.textContent = DIFF_HINT;
    this.body.replaceChildren(element("p", "muted", "reading…"));
    const from = file.kind.type === "renamed" ? file.kind.from : null;
    let diff;
    try {
      diff = await invoke("file_diff", { copy: block.copy, base: block.base, path: file.path, from });
    } catch (error) {
      if (request === this.request) this.body.replaceChildren(element("p", "muted", `${error}`));
      return;
    }
    if (request !== this.request) return;
    const view = element("pre", "change-diff");
    view.tabIndex = 0;
    const lines = diff.length === 0 ? ["no difference"] : diff.replace(/\n$/, "").split("\n");
    const classes = diffClasses(lines);
    // One node per line, added through a fragment: a spread of tens of thousands would pass the
    // engine's limit on arguments.
    const fragment = document.createDocumentFragment();
    lines.forEach((line, index) => fragment.append(element("div", classes[index], line || " ")));
    view.append(fragment);
    this.body.replaceChildren(view);
    view.focus({ preventScroll: true });
  }

  onKey(event) {
    if (this.mode !== "list") return;
    if (event.key === "Enter") {
      event.preventDefault();
      event.stopPropagation();
      this.showDiff();
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    event.stopPropagation();
    if (this.entries.length === 0) return;
    const step = event.key === "ArrowDown" ? 1 : -1;
    this.selected = Math.min(Math.max(this.selected + step, 0), this.entries.length - 1);
    this.render();
  }
}
