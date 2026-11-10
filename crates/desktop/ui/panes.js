// The tabs and panes on the right. Rust owns them (tabs.rs, layout.rs): every change is a
// command, and this module only draws what it gets back.

import { mark, traced } from "./trace.js";

const { invoke, Channel } = window.__TAURI__.core;

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

// Sets a text only when it changes: a write, even of the same text, makes the page lay out again.
function setText(node, text) {
  if (node.textContent !== text) node.textContent = text;
}

// What a tab's page is built from: its panes and splits, not the sessions shown in them.
function shapeOf(node) {
  return node.kind === "leaf" ? node.id : [node.direction, node.ratio, shapeOf(node.first), shapeOf(node.second)];
}

// A file path as a terminal pastes it on a drop: a `\` before any character that is not a letter,
// a digit or plainly safe, `\` of Windows paths included. Claude Code takes the escapes out again,
// and attaches the file as an image when its name ends like one.
export function escapedPath(path) {
  return path.replace(/[^\p{L}\p{N}._/~+\-:@,=]/gu, (character) => `\\${character}`);
}

export function sameSession(key, session) {
  return key !== null && key.account === session.account && key.short_id === session.short_id;
}

// The terminal colors come from the art direction in style.css, so a theme changes them too.
export function terminalOptions() {
  const style = getComputedStyle(document.documentElement);
  const variable = (name) => style.getPropertyValue(name).trim();
  return {
    fontFamily: variable("--font"),
    fontSize: 13,
    theme: {
      background: variable("--terminal"),
      foreground: variable("--terminal-text"),
      cursor: variable("--waiting"),
      cursorAccent: variable("--terminal"),
      selectionBackground: variable("--selection"),
    },
  };
}

const IMAGE_TYPES = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp" };

export function imageType(path) {
  return IMAGE_TYPES[path.split(".").pop().toLowerCase()] ?? "";
}

function fileName(path) {
  return path.split(/[\\/]/).pop();
}

export function loadWebgl(terminal) {
  try {
    const webgl = new WebglAddon.WebglAddon();
    webgl.onContextLoss(() => {
      mark("webgl context lost", performance.now());
      webgl.dispose();
    });
    terminal.loadAddon(webgl);
  } catch (error) {
    mark("webgl unavailable", performance.now());
    console.warn("WebGL unavailable, using the DOM renderer:", error);
  }
}

// One pane: a terminal kept across renders, and the session attached in it. `current.output`
// tells its bytes apart from those of the session shown before, `current.generation` its end.
class PaneView {
  constructor(id, keyFilter) {
    this.id = id;
    this.current = null;
    // A session can end before `open_session` has returned its generation.
    this.lastEnded = null;
    this.used = false;
    // Between opening a session and its first output: the pane says so instead of staying blank.
    this.waiting = false;
    this.element = element("div", "pane");
    this.title = element("span", "head-title");
    this.account = element("span", "head-account");
    this.done = element("span", "head-done", "✓ done");
    this.progress = element("span", "head-progress");
    const head = element("div", "pane-head");
    head.append(element("span", "prompt", "~ $"), this.title, this.account, this.progress, this.done);
    this.screen = element("div", "screen");
    // Over the top of the terminal, which keeps its size: Claude Code does not draw again.
    this.gallery = element("div", "pane-images");
    this.gallery.hidden = true;
    this.screen.append(this.gallery);
    this.shownImages = "";
    this.imageUrls = [];
    this.placeholder = element("p", "placeholder", "select a session");
    this.element.append(head, this.screen, this.placeholder);
    this.terminal = new Terminal(terminalOptions());
    this.fit = new FitAddon.FitAddon();
    this.terminal.loadAddon(this.fit);
    this.terminal.attachCustomKeyEventHandler(keyFilter);
    this.terminal.onData((data) => this.forward("write_input", { data }));
    this.terminal.onResize(({ cols, rows }) => this.forward("resize_pane", { columns: cols, rows }));
    this.observer = new ResizeObserver(() => this.fit.fit());
    this.opened = false;
  }

  // xterm measures its cells when opened: only once the pane is in the page.
  openOnce() {
    if (this.opened) return;
    this.opened = true;
    this.terminal.open(this.screen);
    loadWebgl(this.terminal);
    this.observer.observe(this.screen);
    this.fit.fit();
  }

  // The head names the session shown, and says when it finished a turn not looked at yet.
  label(key, title, finished, progress) {
    setText(this.title, key === null ? "empty" : title);
    setText(this.account, key === null ? "" : `· ${key.account}`);
    setText(this.progress, key === null || !progress ? "" : `· ${progress}`);
    this.progress.title = progress || "";
    this.done.hidden = !finished;
    this.element.classList.toggle("finished", finished);
  }

  // The images the session's last turn proposed; none hides the band.
  async showImages(paths) {
    const shown = paths.join("\n");
    if (shown === this.shownImages) return;
    this.shownImages = shown;
    for (const url of this.imageUrls) URL.revokeObjectURL(url);
    this.imageUrls = [];
    this.gallery.replaceChildren();
    this.gallery.hidden = paths.length === 0;
    const loaded = await Promise.all(
      paths.map((path) =>
        invoke("image_bytes", { path }).then(
          (bytes) => ({ path, bytes }),
          (error) => ({ path, error: `${error}` }),
        ),
      ),
    );
    // Another list came while these were read.
    if (this.shownImages !== shown) return;
    for (const { path, bytes, error } of loaded) {
      if (error !== undefined) {
        this.gallery.append(element("p", "image-error", error));
        continue;
      }
      const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: imageType(path) }));
      this.imageUrls.push(url);
      const image = element("img");
      image.src = url;
      image.alt = fileName(path);
      image.title = path;
      this.gallery.append(image);
    }
  }

  forward(command, args) {
    if (!this.current || this.current.ended) return;
    invoke(command, { pane: this.id, ...args }).catch((error) => console.error(error));
  }

  connecting(title) {
    this.waiting = true;
    setText(this.placeholder, `connecting to ${title}…`);
    this.placeholder.hidden = false;
  }

  connected() {
    if (!this.waiting) return;
    this.waiting = false;
    this.placeholder.hidden = true;
    setText(this.placeholder, "select a session");
  }

  showEnd() {
    this.connected();
    this.current.ended = true;
    this.terminal.write("\r\n\x1b[2m[session closed]\x1b[0m\r\n");
  }

  dispose() {
    for (const url of this.imageUrls) URL.revokeObjectURL(url);
    this.observer.disconnect();
    this.terminal.dispose();
  }
}

function leaves(node, found = []) {
  if (node.kind === "leaf") found.push(node);
  else {
    leaves(node.first, found);
    leaves(node.second, found);
  }
  return found;
}

export class Panes {
  constructor(container, bar, { keyFilter, onChange, onError, titleOf, isFinished, markOf, progressOf, imagesOf }) {
    this.container = container;
    this.bar = bar;
    this.keyFilter = keyFilter;
    this.onChange = onChange;
    this.onError = onError;
    this.titleOf = titleOf;
    this.isFinished = isFinished;
    this.markOf = markOf;
    this.progressOf = progressOf;
    this.imagesOf = imagesOf;
    this.state = null;
    this.views = new Map();
    // Tab id → its page and the shape it was built for.
    this.pages = new Map();
    this.tabDrag = false;
    this.tabDropped = -Infinity;
    // Terminals moved in the page since the last render: moving one loses what it drew.
    this.moved = [];
    // Where the mouse was last seen in the window, or null once it left: a split puts the new
    // pane under it.
    this.pointer = null;
    document.addEventListener("mousemove", (event) => (this.pointer = { x: event.clientX, y: event.clientY }), true);
    document.documentElement.addEventListener("mouseleave", () => (this.pointer = null));
  }

  // The terminals take the colors of the theme just applied.
  retheme() {
    const { theme } = terminalOptions();
    for (const view of this.views.values()) view.terminal.options.theme = theme;
  }

  async load() {
    this.render(await invoke("tabs"));
  }

  // The visible tab: its `project` is null for "other".
  activeTab() {
    return this.state.tabs.find((tab) => tab.id === this.state.active);
  }

  layout() {
    return this.activeTab().layout;
  }

  // The session shown in the active pane of the visible tab, or null.
  activeSession() {
    const layout = this.layout();
    const leaf = leaves(layout.root).find((node) => node.id === layout.active);
    return leaf ? leaf.session : null;
  }

  projectName(id) {
    const tab = this.state === null ? null : this.state.tabs.find((tab) => tab.id === id);
    return tab && tab.project ? tab.project.name : null;
  }

  // No pane of `tab` shows a session.
  tabEmpty(tab) {
    return leaves(tab.layout.root).every((leaf) => leaf.session === null);
  }

  allLeaves() {
    return this.state.tabs.flatMap((tab) => leaves(tab.layout.root));
  }

  view(id) {
    if (!this.views.has(id)) this.views.set(id, new PaneView(id, this.keyFilter));
    return this.views.get(id);
  }

  active() {
    return this.view(this.layout().active);
  }

  // Whether a pane of the visible tab shows `session`: opened from another tab, it moves.
  shows(session) {
    return this.state !== null && leaves(this.layout().root).some((leaf) => sameSession(leaf.session, session));
  }

  draw(node) {
    if (node.kind === "leaf") {
      const view = this.view(node.id);
      view.element.onmousedown = () => this.focusPane(node.id);
      // The pane under the mouse is the active one, unless a button is held (a selection, a
      // separator or a tab being dragged) or a dialog is open over the panes.
      view.element.onmouseenter = (event) => {
        if (event.buttons === 0 && !document.querySelector("dialog[open]")) this.focusPane(node.id, "hover");
      };
      return view.element;
    }
    const split = element("div", `split ${node.direction}`);
    const first = this.draw(node.first);
    const second = this.draw(node.second);
    first.style.flex = `${node.ratio} 1 0px`;
    second.style.flex = `${1 - node.ratio} 1 0px`;
    const divider = element("div", "divider");
    divider.onmousedown = (event) => this.drag(event, node, split, first, second);
    split.append(first, divider, second);
    return split;
  }

  // The page of a tab, built again only when its panes or splits change: otherwise its
  // terminals stay where they are, since moving one makes it measure and draw again.
  page(tab) {
    const shape = JSON.stringify(shapeOf(tab.layout.root));
    let built = this.pages.get(tab.id);
    if (!built || built.shape !== shape) {
      const page = built ? built.page : element("div", "tab-page");
      const root = this.draw(tab.layout.root);
      // A pane lifted to the root after a close still has the share it had in its split.
      root.style.flex = "";
      page.replaceChildren(root);
      built = { page, shape };
      this.pages.set(tab.id, built);
      this.moved.push(...leaves(tab.layout.root).map((leaf) => this.view(leaf.id)));
    }
    for (const leaf of leaves(tab.layout.root)) {
      const view = this.view(leaf.id);
      view.element.classList.toggle("active", leaf.id === tab.layout.active);
      // Emptied after its session opened (hidden, say): nothing is coming any more.
      if (leaf.session === null && view.current && view.current.generation !== null) view.connected();
      view.placeholder.hidden = !view.waiting && (leaf.session !== null || view.used);
    }
    built.page.hidden = tab.id !== this.state.active;
    return built.page;
  }

  // Every tab stays in the page, hidden ones with `display: none`: their terminals keep
  // receiving output.
  render(state) {
    const start = performance.now();
    this.state = state;
    const ids = new Set(this.allLeaves().map((leaf) => leaf.id));
    for (const [id, view] of this.views) {
      if (!ids.has(id)) {
        view.dispose();
        this.views.delete(id);
      }
    }
    const tabIds = new Set(state.tabs.map((tab) => tab.id));
    for (const id of this.pages.keys()) {
      if (!tabIds.has(id)) this.pages.delete(id);
    }
    const pages = state.tabs.map((tab) => this.page(tab));
    const shown = [...this.container.children];
    if (pages.length !== shown.length || pages.some((page, index) => page !== shown[index])) {
      this.container.replaceChildren(...pages);
    }
    // xterm measures its cells when opened, so only panes on screen are opened; every pane
    // is created in the active tab.
    for (const leaf of leaves(this.layout().root)) this.view(leaf.id).openOnce();
    for (const view of this.moved) {
      if (view.opened) view.terminal.refresh(0, view.terminal.rows - 1);
    }
    this.moved = [];
    this.renderBar();
    this.decorate();
    this.focus();
    this.onChange();
    mark("render panes", start);
  }

  // Names the session of every pane, and marks those that finished a turn not looked at.
  decorate() {
    if (this.state === null) return;
    for (const leaf of this.allLeaves()) {
      const key = leaf.session;
      const title = key === null ? "" : this.titleOf(key);
      const view = this.view(leaf.id);
      view.label(key, title, key !== null && this.isFinished(key), key === null ? null : this.progressOf(key));
      view.showImages(key === null ? [] : this.imagesOf(key));
    }
  }

  // "n name" per tab, which carries the dot of its most urgent session.
  renderBar() {
    // A tab being dragged keeps the bar as it is until it is dropped.
    if (this.state === null || this.tabDrag) return;
    this.bar.replaceChildren(
      ...this.state.tabs.map((tab, index) => {
        const button = element("button", "tab");
        button.append(element("span", "pos", `${index + 1}`), element("span", "tab-name", tab.project ? tab.project.name : "other"));
        button.title = tab.project ? tab.project.directory : "sessions of no project";
        if (tab.project) {
          // Read at each hover, before the tooltip shows: a zone declared since shows at once.
          button.onmouseenter = async () => {
            const memory = await invoke("memory_zone", { directory: tab.project.directory }).catch((error) => `memory not read: ${error}`);
            button.title = `${tab.project.directory}\n${memory}`;
          };
        }
        const mark = this.markOf(tab.id);
        if (mark) {
          button.classList.add(mark);
          button.append(element("span", "dot", mark === "finished" ? "✓" : ""));
        }
        button.classList.toggle("active", tab.id === this.state.active);
        // The click that ends a drag is not a click on the tab.
        button.onclick = () => performance.now() - this.tabDropped > 300 && this.goTab(index + 1);
        if (tab.project) button.onmousedown = (event) => this.dragTab(event, button, index);
        return button;
      }),
    );
  }

  focus() {
    if (!document.querySelector("dialog[open]")) this.active().terminal.focus();
  }

  // Files dropped from outside the window, at (x, y) in the page: their paths go to the session of
  // the pane under the pointer, else of the active pane. One paste each, so that Claude Code still
  // recognizes an image among several files.
  async dropFiles(paths, x, y) {
    const under = document.elementFromPoint(x, y);
    const leaf = leaves(this.layout().root).find((node) => this.view(node.id).element.contains(under));
    const id = leaf?.id ?? this.layout().active;
    const view = this.view(id);
    if (!view.current || view.current.ended) {
      this.onError("no session in this pane to drop files into");
      return;
    }
    await this.focusPane(id);
    for (const path of paths) view.terminal.paste(`${escapedPath(path)} `);
    view.terminal.focus();
  }

  // A project tab follows the mouse along the bar, among the other project tabs: "other"
  // stays first. A press that does not move is a click. Rust gets the new place on release.
  dragTab(event, button, from) {
    if (event.button !== 0) return;
    const startX = event.clientX;
    let to = from;
    const move = (moved) => {
      if (!this.tabDrag && Math.abs(moved.clientX - startX) < 5) return;
      this.tabDrag = true;
      button.classList.add("dragging");
      const others = [...this.bar.children].slice(1).filter((other) => other !== button);
      const before = others.find((other) => {
        const box = other.getBoundingClientRect();
        return moved.clientX < box.left + box.width / 2;
      });
      this.bar.insertBefore(button, before ?? null);
      to = [...this.bar.children].indexOf(button);
    };
    const release = async () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", release);
      if (!this.tabDrag) return;
      this.tabDropped = performance.now();
      this.tabDrag = false;
      if (to === from) {
        this.renderBar();
        return;
      }
      try {
        this.render(await invoke("move_tab", { from: from + 1, to: to + 1 }));
      } catch (error) {
        this.renderBar();
        this.onError(`${error}`);
      }
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", release);
  }

  // The separator follows the mouse; Rust gets the ratio once it is released.
  drag(event, node, split, first, second) {
    event.preventDefault();
    const row = node.direction === "row";
    let ratio = node.ratio;
    const start = performance.now();
    let moves = 0;
    const move = (moved) => {
      moves += 1;
      const box = split.getBoundingClientRect();
      const offset = row ? moved.clientX - box.left : moved.clientY - box.top;
      ratio = Math.min(Math.max(offset / (row ? box.width : box.height), 0.1), 0.9);
      first.style.flex = `${ratio} 1 0px`;
      second.style.flex = `${1 - ratio} 1 0px`;
    };
    const release = async () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", release);
      mark(`drag split (${moves} moves)`, start);
      this.render(await invoke("resize_split", { split: node.id, ratio }));
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", release);
  }

  async focusPane(id, cause = "click") {
    if (id !== this.layout().active) await traced(`focus pane (${cause})`, async () => this.render(await invoke("focus_pane", { pane: id })));
  }

  async step(step) {
    this.render(await invoke("focus_step", { step }));
  }

  async split() {
    const box = this.active().element.getBoundingClientRect();
    const pointer = this.pointer && [this.pointer.x - box.left, this.pointer.y - box.top];
    this.render(await invoke("split_pane", { width: box.width, height: box.height, pointer }));
  }

  async close() {
    this.render(await invoke("close_pane"));
  }

  async addProject(directory) {
    this.render(await invoke("add_project", { directory }));
  }

  async closeTab() {
    this.render(await invoke("close_tab"));
  }

  async tabStep(step) {
    this.render(await invoke("tab_step", { step }));
  }

  async goTab(number) {
    this.render(await invoke("go_tab", { number }));
  }

  async goTabId(id) {
    const index = this.state.tabs.findIndex((tab) => tab.id === id);
    if (index >= 0 && id !== this.state.active) await this.goTab(index + 1);
  }

  // Shows `session` in the active pane. If another pane of this tab shows it already, Rust
  // gives that pane the focus instead; a pane of another tab gives it up.
  async open(session) {
    const start = performance.now();
    const view = this.active();
    const request = {
      pane: view.id,
      account: session.account,
      shortId: session.short_id,
      columns: view.terminal.cols,
      rows: view.terminal.rows,
    };
    if (this.shows(session)) {
      const opened = await invoke("open_session", { ...request, output: new Channel() });
      this.render(opened.tabs);
      return;
    }
    const output = new Channel();
    const opening = { output, generation: null, ended: false };
    view.current = opening;
    view.used = true;
    let shown = false;
    output.onmessage = (bytes) => {
      if (view.current !== opening) return;
      view.connected();
      view.terminal.write(new Uint8Array(bytes), shown ? undefined : () => mark("first output shown", start));
      shown = true;
    };
    view.terminal.reset();
    view.connecting(this.titleOf(session));
    let opened;
    try {
      opened = await invoke("open_session", { ...request, output });
    } catch (error) {
      if (view.current !== opening) return;
      view.showEnd();
      view.terminal.write(`\x1b[31m${error}\x1b[0m\r\n`);
      return;
    }
    // Null: another pane showed it already (this page had not seen it yet) and has the focus.
    if (opened.generation === null && view.current === opening) {
      view.current = null;
      view.connected();
    }
    opening.generation = opened.generation;
    if (view.current === opening && view.lastEnded === opening.generation) view.showEnd();
    this.render(opened.tabs);
  }

  ended({ pane, generation }) {
    const view = this.views.get(pane);
    if (!view) return;
    view.lastEnded = generation;
    if (view.current && view.current.generation === generation) view.showEnd();
  }
}
