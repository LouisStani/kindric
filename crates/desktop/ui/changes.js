// The changed files window (Ctrl+Shift+G, or a click on "N files" in a pane's head): what the
// session of the active pane changed, one block per working copy, in the order the session went
// into them. Rust computes the blocks with git (changes.rs) when the window opens.

const { invoke } = window.__TAURI__.core;

const HINT = "↑↓ move · esc close";
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

export class ChangesWindow {
  constructor(dialog) {
    this.dialog = dialog;
    this.title = dialog.querySelector(".changes-title");
    this.body = dialog.querySelector(".changes-body");
    this.hint = dialog.querySelector(".changes-hint");
    this.blocks = [];
    // Every file in the order shown, across the blocks: what ↑↓ moves over.
    this.files = [];
    this.selected = 0;
    this.request = 0;
    dialog.addEventListener("keydown", (event) => this.onKey(event));
  }

  async open(account, shortId, label) {
    const request = ++this.request;
    this.title.textContent = `changed files · ${label}`;
    this.hint.textContent = HINT;
    this.body.replaceChildren(element("p", "muted", "counting…"));
    this.files = [];
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
    this.files = blocks.flatMap((block) => block.files ?? []);
    this.render();
  }

  render() {
    if (this.files.length === 0 && this.blocks.every((block) => block.type === "changes")) {
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
    const path = file.kind.type === "renamed" ? `${file.kind.from} → ${file.path}` : file.path;
    const name = element("span", "artifact-name", path);
    row.title = path;
    row.append(element("span", `change-kind kind-${file.kind.type}`, LETTERS[file.kind.type] ?? "?"), name, ...counts(file));
    row.onclick = () => {
      this.selected = index;
      this.render();
    };
    return row;
  }

  onKey(event) {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    event.stopPropagation();
    if (this.files.length === 0) return;
    const step = event.key === "ArrowDown" ? 1 : -1;
    this.selected = Math.min(Math.max(this.selected + step, 0), this.files.length - 1);
    this.render();
  }
}
