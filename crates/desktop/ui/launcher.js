// Quick launch (decision 272): in a project, a list of the accounts; Enter or a click starts a
// session in the project's folder at once, with no name. The last line opens the full form.
// The same list is shown in a dialog (Ctrl+N) and over the panes of a project with no session.

const FORM = "other folder or name…";

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

// The account of the project's last session first, while it still exists; the others as given.
export function launchOrder(accounts, preferred) {
  if (!preferred || !accounts.includes(preferred)) return [...accounts];
  return [preferred, ...accounts.filter((account) => account !== preferred)];
}

export class AccountList {
  // `labelsOf(accounts)` gives a Map of each account to its line; `onStart(account, project)`
  // starts the session and throws to say why it could not; `onForm()` opens the full form.
  constructor(root, { labelsOf, onStart, onForm, onTrust }) {
    this.root = root;
    this.list = root.querySelector(".launch-list");
    this.message = root.querySelector(".launch-message");
    this.labelsOf = labelsOf;
    this.onStart = onStart;
    this.onForm = onForm;
    this.onTrust = onTrust;
    this.project = null;
    this.accounts = [];
    this.labels = new Map();
    this.selected = 0;
    this.starting = false;
    // What the list was last drawn for, so that a render with nothing new keeps the selection.
    this.shownFor = "";
    root.addEventListener("keydown", (event) => {
      if (this.onKey(event.key)) {
        event.preventDefault();
        event.stopPropagation();
      }
    });
  }

  // Draws the list for `project`; the same project and accounts keep the selection and message.
  async show(project, accounts) {
    const order = launchOrder(accounts, project.account);
    const shownFor = JSON.stringify([project.directory, order]);
    if (shownFor === this.shownFor) return;
    this.shownFor = shownFor;
    this.project = project;
    this.accounts = order;
    this.selected = 0;
    this.message.replaceChildren();
    this.render();
    const labels = await this.labelsOf(order);
    if (this.shownFor !== shownFor) return;
    this.labels = labels;
    this.render();
  }

  // The next `show` draws the list again, from the first account.
  forget() {
    this.shownFor = "";
  }

  rows() {
    return [...this.accounts.map((account) => ({ account, label: this.labels.get(account) ?? account })), { form: true, label: FORM }];
  }

  render() {
    const rows = this.rows();
    this.list.replaceChildren(
      ...rows.map((row, index) => {
        const item = element("li", row.form ? "launch-form muted" : "", row.label);
        item.classList.toggle("selected", index === this.selected);
        item.onclick = () => {
          this.selected = index;
          this.choose();
        };
        return item;
      }),
    );
  }

  onKey(key) {
    const last = this.accounts.length;
    if (key === "ArrowDown" || key === "ArrowUp") {
      const step = key === "ArrowDown" ? 1 : -1;
      this.selected = Math.min(Math.max(this.selected + step, 0), last);
      this.render();
    } else if (key === "Enter") this.choose();
    else return false;
    return true;
  }

  async choose() {
    const row = this.rows()[this.selected];
    if (!row || this.starting) return;
    if (row.form) {
      this.onForm();
      return;
    }
    const project = this.project;
    this.starting = true;
    this.message.textContent = `starting a session on ${row.account}…`;
    try {
      await this.onStart(row.account, project);
      this.message.replaceChildren();
    } catch (error) {
      this.say(error, row.account, project);
    } finally {
      this.starting = false;
    }
  }

  // Claude Code refused a folder the account has not trusted yet: it asks, in its own window,
  // and the session is started again once that window is closed.
  say(error, account, project) {
    if (!error.trust) {
      this.message.textContent = error.message ?? `${error}`;
      return;
    }
    const trust = element("button", "", "trust it");
    trust.type = "button";
    trust.onclick = () => this.onTrust(account, project.directory, () => this.choose());
    this.message.replaceChildren(`${account} has not trusted this folder yet.`, trust);
  }
}
