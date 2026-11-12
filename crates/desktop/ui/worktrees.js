// The worktrees window (Ctrl+Shift+B): the worktrees of the project of the visible tab, which are
// merged into the branch of the main checkout, and removing one with Delete then Enter. Rust asks
// git (worktrees.rs); it refuses again what this window refuses.

const { invoke } = window.__TAURI__.core;

const LIST_HINT = "↑↓ move · delete remove · esc close";
const CONFIRM_HINT = "enter remove · esc cancel";
// The states that cannot be removed, with why; Rust holds the same list.
const REFUSALS = {
  main: () => "the main checkout is never removed",
  busy: (state) => `the session ${state.session} works in it`,
  missing: () => "its folder is already gone: git worktree prune forgets it",
  locked: () => "it is locked: git worktree unlock first",
  uncommitted: () => "it has uncommitted changes",
  failed: (state) => state.error,
};

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function stateText(state) {
  switch (state.type) {
    case "main": return "main checkout";
    case "busy": return `a session works here: ${state.session}`;
    case "missing": return "folder gone";
    case "locked": return state.reason ? `locked: ${state.reason}` : "locked";
    case "uncommitted": return "uncommitted changes";
    case "merged": return "merged";
    case "merged-squash": return "merged (squash)";
    case "not-merged": return `not merged · ${state.ahead} commit${state.ahead === 1 ? "" : "s"} ahead`;
    case "no-branch": return "no branch";
    default: return state.error ?? state.type;
  }
}

function ago(seconds) {
  const age = Math.max(0, Date.now() / 1000 - seconds);
  if (age < 3600) return `${Math.floor(age / 60)}m ago`;
  if (age < 172800) return `${Math.floor(age / 3600)}h ago`;
  return `${Math.floor(age / 86400)}d ago`;
}

// The folder name for a worktree, the path from the main checkout's for others outside it.
function nameOf(worktree, main) {
  const path = worktree.path;
  if (worktree.state.type === "main") return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  if (main && path.startsWith(main.path)) {
    const inside = path.slice(main.path.length).replace(/^[\\/]+/, "");
    return inside.replace(/^\.claude[\\/]worktrees[\\/]/, "");
  }
  return path;
}

// What removing it does with its branch, said before Enter.
function branchFate(worktree) {
  if (!worktree.branch) return "";
  const kept = worktree.on_origin ? ", the one on origin is not touched" : "";
  if (worktree.state.type === "merged") return ` and its branch ${worktree.branch}${kept}`;
  return `; its branch ${worktree.branch} stays (git deletes only a merged branch)${kept}`;
}

export class WorktreesWindow {
  constructor(dialog) {
    this.dialog = dialog;
    this.title = dialog.querySelector(".worktrees-title");
    this.body = dialog.querySelector(".worktrees-body");
    this.note = dialog.querySelector(".worktrees-note");
    this.hint = dialog.querySelector(".worktrees-hint");
    this.project = null;
    this.worktrees = [];
    this.selected = 0;
    // "list", "confirm" (Delete pressed, waiting for Enter) or "removing".
    this.mode = "list";
    this.request = 0;
    dialog.addEventListener("keydown", (event) => this.onKey(event));
    // Escape cancels a removal asked for; from the list it closes the window.
    dialog.addEventListener("cancel", (event) => {
      if (this.mode === "list") return;
      event.preventDefault();
      if (this.mode === "confirm") this.cancel();
    });
  }

  // `project`: the tab's folder, null for "other".
  async open(project, label) {
    this.project = project;
    this.mode = "list";
    this.selected = 0;
    this.worktrees = [];
    this.title.textContent = `worktrees · ${label}`;
    this.hint.textContent = LIST_HINT;
    this.note.textContent = "";
    this.dialog.showModal();
    this.dialog.focus({ preventScroll: true });
    if (!project) {
      this.body.replaceChildren(element("p", "muted", "no project in this tab"));
      return;
    }
    await this.load();
  }

  async load() {
    const request = ++this.request;
    if (this.worktrees.length === 0) this.body.replaceChildren(element("p", "muted", "asking git…"));
    let worktrees;
    try {
      worktrees = await invoke("project_worktrees", { project: this.project });
    } catch (error) {
      if (request === this.request) this.body.replaceChildren(element("p", "muted", `${error}`));
      return;
    }
    if (request !== this.request) return;
    this.worktrees = worktrees;
    this.selected = Math.min(this.selected, Math.max(worktrees.length - 1, 0));
    this.render();
  }

  render() {
    const main = this.worktrees[0];
    const list = element("ol", "artifacts-list");
    this.worktrees.forEach((worktree, index) => list.append(this.item(worktree, index, main)));
    this.body.replaceChildren(list);
    if (this.worktrees.length === 1) this.body.append(element("p", "muted", "no worktree besides the main checkout"));
    list.querySelector(".selected")?.scrollIntoView({ block: "nearest" });
  }

  item(worktree, index, main) {
    const row = element("li", "artifact worktree");
    row.classList.toggle("selected", index === this.selected);
    row.title = worktree.path;
    const name = element("span", "artifact-name", nameOf(worktree, main));
    if (worktree.branch) name.append(element("span", "worktree-branch", ` ${worktree.branch}`));
    const parts = [stateText(worktree.state)];
    if (worktree.on_origin) parts.push("also on origin");
    row.append(name, element("span", `worktree-state state-${worktree.state.type}`, parts.join(" · ")));
    if (worktree.last_commit) row.append(element("span", "artifact-date", ago(worktree.last_commit)));
    row.onclick = () => {
      if (this.mode !== "list") return;
      this.selected = index;
      this.render();
    };
    return row;
  }

  ask() {
    const worktree = this.worktrees[this.selected];
    if (!worktree) return;
    const refusal = REFUSALS[worktree.state.type];
    if (refusal) {
      this.note.textContent = `cannot remove ${nameOf(worktree, this.worktrees[0])}: ${refusal(worktree.state)}`;
      return;
    }
    this.mode = "confirm";
    this.note.textContent = `remove the folder ${worktree.path}${branchFate(worktree)}?`;
    this.hint.textContent = CONFIRM_HINT;
  }

  cancel() {
    this.mode = "list";
    this.note.textContent = "";
    this.hint.textContent = LIST_HINT;
  }

  async remove() {
    const worktree = this.worktrees[this.selected];
    const name = nameOf(worktree, this.worktrees[0]);
    this.mode = "removing";
    this.note.textContent = `removing ${name}…`;
    let outcome;
    try {
      outcome = await invoke("remove_worktree", { project: this.project, path: worktree.path });
    } catch (error) {
      this.cancel();
      this.note.textContent = `cannot remove ${name}: ${error}`;
      return;
    }
    this.cancel();
    if (outcome.type === "deleted") this.note.textContent = `removed ${name} and its branch ${worktree.branch}`;
    else if (outcome.type === "kept") this.note.textContent = `removed ${name}; branch ${worktree.branch} kept: ${outcome.reason}`;
    else this.note.textContent = `removed ${name}`;
    await this.load();
  }

  onKey(event) {
    if (this.mode === "removing") {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    if (this.mode === "confirm") {
      if (event.key !== "Enter") return;
      event.preventDefault();
      event.stopPropagation();
      this.remove();
      return;
    }
    if (event.key === "Delete") {
      event.preventDefault();
      event.stopPropagation();
      this.ask();
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    event.stopPropagation();
    if (this.worktrees.length === 0) return;
    const step = event.key === "ArrowDown" ? 1 : -1;
    this.selected = Math.min(Math.max(this.selected + step, 0), this.worktrees.length - 1);
    this.note.textContent = "";
    this.render();
  }
}
