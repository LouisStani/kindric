// Claude Code in a terminal over a dialog, for what only Claude Code itself can do: ask whether a
// folder is trusted (`claude --bg` refuses one the account has not trusted), or sign an account
// in. Claude Code keeps the answer; this window only shows it and passes the keys.

import { loadUnicode11, loadWebgl, terminalOptions } from "./panes.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// Opens the window with its texts, then starts Claude Code with `open(columns, rows, output)`, a
// command that resolves to its generation; `output` numbers the terminal in `outbox`. `onClosed`
// runs once the window is closed, by "done", by Escape, or because Claude Code ended by itself.
async function runHelper(outbox, { title, hint, subject }, open, onClosed) {
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
  const output = outbox.open((bytes) => terminal.write(bytes));
  const input = terminal.onData((data) => invoke("helper_input", { data }).catch(() => {}));
  const resize = terminal.onResize(({ cols, rows }) => invoke("helper_resize", { columns: cols, rows }).catch(() => {}));
  dialog.addEventListener(
    "close",
    async () => {
      unlisten();
      outbox.close(output);
      input.dispose();
      resize.dispose();
      await invoke("close_helper");
      terminal.dispose();
      onClosed();
    },
    { once: true },
  );

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
export function askTrust(outbox, account, directory, onClosed) {
  return runHelper(
    outbox,
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
export function logIn(outbox, account, onClosed) {
  return runHelper(
    outbox,
    {
      title: "log in",
      hint: "follow Claude Code below, then press done.",
      subject: account,
    },
    (columns, rows, output) => invoke("open_login", { account, columns, rows, output }),
    onClosed,
  );
}
