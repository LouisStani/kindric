// Claude Code in a terminal over a dialog, for what only Claude Code itself can do: ask whether a
// folder is trusted (`claude --bg` refuses one the account has not trusted), or sign an account
// in. Claude Code keeps the answer; this window only shows it and passes the keys.

import { loadUnicode11, loadWebgl, terminalOptions } from "./panes.js";

const { invoke, Channel } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// Opens the window with its texts, then starts Claude Code with `open(columns, rows, output)`, a
// command that resolves to its generation. `onClosed` runs once the window is closed, by "done",
// by Escape, or because Claude Code ended by itself.
async function runHelper({ title, hint, subject }, open, onClosed) {
  const dialog = document.getElementById("helper");
  const screen = document.getElementById("helper-screen");
  const message = document.getElementById("helper-message");
  document.getElementById("helper-title").textContent = title;
  document.getElementById("helper-hint").textContent = hint;
  document.getElementById("helper-subject").textContent = subject;
  message.textContent = "";
  screen.replaceChildren();
  const terminal = new Terminal(terminalOptions());
  loadUnicode11(terminal);
  const fit = new FitAddon.FitAddon();
  terminal.loadAddon(fit);
  dialog.showModal();
  terminal.open(screen);
  loadWebgl(terminal);
  fit.fit();

  // Claude Code can end before `open` has returned its generation.
  let generation = null;
  let lastEnded = null;
  const unlisten = await listen("helper-ended", (event) => {
    lastEnded = event.payload;
    if (lastEnded === generation) dialog.close();
  });
  const input = terminal.onData((data) => invoke("helper_input", { data }).catch(() => {}));
  const resize = terminal.onResize(({ cols, rows }) => invoke("helper_resize", { columns: cols, rows }).catch(() => {}));
  dialog.addEventListener(
    "close",
    async () => {
      unlisten();
      input.dispose();
      resize.dispose();
      await invoke("close_helper");
      terminal.dispose();
      onClosed();
    },
    { once: true },
  );

  const output = new Channel();
  output.onmessage = (bytes) => terminal.write(new Uint8Array(bytes));
  try {
    generation = await open(terminal.cols, terminal.rows, output);
  } catch (error) {
    message.textContent = `${error}`;
    return;
  }
  if (lastEnded === generation) dialog.close();
  else terminal.focus();
}

// Claude Code asks whether `account` trusts `directory`.
export function askTrust(account, directory, onClosed) {
  return runHelper(
    {
      title: "trust this folder",
      hint: "answer Claude Code below, then press done: kindric starts the session again.",
      subject: `${directory} · ${account}`,
    },
    (columns, rows, output) => invoke("open_trust", { account, directory, columns, rows, output }),
    onClosed,
  );
}

// Claude Code signs `account` in.
export function logIn(account, onClosed) {
  return runHelper(
    {
      title: "log in",
      hint: "follow Claude Code below, then press done.",
      subject: account,
    },
    (columns, rows, output) => invoke("open_login", { account, columns, rows, output }),
    onClosed,
  );
}
