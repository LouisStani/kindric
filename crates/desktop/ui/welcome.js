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

// One item per line: its state in words, its text, what to copy, and "start one" for the
// first session.
export function checkItems(lines, onStart) {
  return lines.map((line) => {
    const item = element("li", `check ${line.state.replace(" ", "-")}`);
    item.append(element("span", "check-state", line.state), element("span", "check-text", line.text));
    for (const text of line.copy) item.append(copyRow(text));
    if (line.start) {
      const button = element("button", "check-start", "start one");
      button.type = "button";
      button.addEventListener("click", onStart);
      item.append(button);
    }
    return item;
  });
}

// A warning alone does not keep the welcome screen up.
export function hasToDo(lines) {
  return lines !== null && lines.some((line) => line.state === "to do");
}
