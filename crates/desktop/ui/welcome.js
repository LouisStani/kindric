// The welcome list: what is set up and what is left to do. Rust decides every line
// (checks.rs); this module only draws them.

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

// The button says whether the text reached the clipboard. When it did not, the text is
// selected instead, for Ctrl+C.
async function copy(text, code, button) {
  try {
    await navigator.clipboard.writeText(text);
    button.textContent = "copied";
  } catch {
    const range = document.createRange();
    range.selectNodeContents(code);
    window.getSelection().removeAllRanges();
    window.getSelection().addRange(range);
    button.textContent = "selected";
  }
  setTimeout(() => (button.textContent = "copy"), 2000);
}

function copyRow(text) {
  const row = element("div", "check-copy");
  const code = element("code", "", text);
  const button = element("button", "", "copy");
  button.type = "button";
  button.addEventListener("click", () => copy(text, code, button));
  row.append(code, button);
  return row;
}

function actionButton(text, onClick) {
  const button = element("button", "check-action", text);
  button.type = "button";
  button.addEventListener("click", onClick);
  return button;
}

// "add" runs the commands above it. On success the list is drawn again, so this button goes;
// on failure it comes back, with what the command said under it.
function addButton(onAdd) {
  const failure = element("p", "check-error");
  failure.hidden = true;
  const button = actionButton("add", async () => {
    button.disabled = true;
    button.textContent = "adding…";
    failure.hidden = true;
    const error = await onAdd();
    if (error === null) return;
    button.disabled = false;
    button.textContent = "add";
    failure.textContent = error;
    failure.hidden = false;
  });
  return [button, failure];
}

// One item per line: its state in words, its text, what to copy, "add" to run it for you,
// and "start one" for the first session. `onAdd` resolves to null or to an error.
export function checkItems(lines, { onStart, onAdd }) {
  return lines.map((line) => {
    const item = element("li", `check ${line.state.replace(" ", "-")}`);
    item.append(element("span", "check-state", line.state), element("span", "check-text", line.text));
    for (const text of line.copy) item.append(copyRow(text));
    if (line.add) item.append(...addButton(onAdd));
    if (line.start) item.append(actionButton("start one", onStart));
    return item;
  });
}

// A warning alone does not keep the welcome screen up.
export function hasToDo(lines) {
  return lines !== null && lines.some((line) => line.state === "to do");
}
