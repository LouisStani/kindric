// Claude Code asking whether a folder is trusted, in a terminal over the new-session form:
// `claude --bg` refuses a folder the account has not trusted, and only Claude Code itself asks.
// It keeps the answer; this window only shows it and passes the keys.

import { loadWebgl, terminalOptions } from "./panes.js";

const { invoke, Channel } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// Opens the window for `account` and `directory`. `onClosed` runs once it is closed, by "done",
// by Escape, or because Claude Code ended by itself (it was answered no, or quit).
export async function askTrust(account, directory, onClosed) {
  const dialog = document.getElementById("trust");
  const screen = document.getElementById("trust-screen");
  const message = document.getElementById("trust-message");
  document.getElementById("trust-folder").textContent = `${directory} · ${account}`;
  message.textContent = "";
  screen.replaceChildren();
  const terminal = new Terminal(terminalOptions());
  const fit = new FitAddon.FitAddon();
  terminal.loadAddon(fit);
  dialog.showModal();
  terminal.open(screen);
  loadWebgl(terminal);
  fit.fit();

  // Claude Code can end before `open_trust` has returned its generation.
  let generation = null;
  let lastEnded = null;
  const unlisten = await listen("trust-ended", (event) => {
    lastEnded = event.payload;
    if (lastEnded === generation) dialog.close();
  });
  const input = terminal.onData((data) => invoke("trust_input", { data }).catch(() => {}));
  const resize = terminal.onResize(({ cols, rows }) => invoke("trust_resize", { columns: cols, rows }).catch(() => {}));
  dialog.addEventListener(
    "close",
    async () => {
      unlisten();
      input.dispose();
      resize.dispose();
      await invoke("close_trust");
      terminal.dispose();
      onClosed();
    },
    { once: true },
  );

  const output = new Channel();
  output.onmessage = (bytes) => terminal.write(new Uint8Array(bytes));
  try {
    generation = await invoke("open_trust", { account, directory, columns: terminal.cols, rows: terminal.rows, output });
  } catch (error) {
    message.textContent = `${error}`;
    return;
  }
  if (lastEnded === generation) dialog.close();
  else terminal.focus();
}
