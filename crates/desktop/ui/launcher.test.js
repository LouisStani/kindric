// Tests of the list of accounts (decisions 272, 348): `node --test crates/desktop/ui/launcher.test.js`.
// The few parts of the DOM the list draws with are faked below.

import { test } from "node:test";
import assert from "node:assert/strict";
import { AccountList, launchOrder } from "./launcher.js";

globalThis.document = {
  createElement(tag) {
    const classes = new Set();
    return {
      tag,
      className: "",
      textContent: "",
      onclick: null,
      scrolledTo: false,
      scrollIntoView() {
        this.scrolledTo = true;
      },
      classList: {
        toggle: (name, on) => (on ? classes.add(name) : classes.delete(name)),
        contains: (name) => classes.has(name),
      },
    };
  },
};

function fakeRoot() {
  const list = {
    items: [],
    replaceChildren(...items) {
      this.items = items;
    },
  };
  const message = { textContent: "", replaceChildren() {} };
  return {
    list,
    keydown: null,
    querySelector: (selector) => ({ ".launch-list": list, ".launch-message": message })[selector] ?? null,
    addEventListener(type, handler) {
      if (type === "keydown") this.keydown = handler;
    },
  };
}

function key(root, name) {
  root.keydown({ key: name, preventDefault() {}, stopPropagation() {} });
}

function labels(accounts) {
  return Promise.resolve(new Map(accounts.map((account) => [account, `${account} · 5h 10%`])));
}

function picking(onEnter) {
  const root = fakeRoot();
  const list = new AccountList(root, { labelsOf: labels, onEnter });
  return { root, list };
}

function selectedText(root) {
  return root.list.items.filter((item) => item.classList.contains("selected")).map((item) => item.textContent);
}

test("the last account of a project comes first, the others in their order", () => {
  assert.deepEqual(launchOrder(["a", "b", "c"], "c"), ["c", "a", "b"]);
  assert.deepEqual(launchOrder(["a", "b"], "gone"), ["a", "b"]);
});

test("a list that only picks has no line to the form", async () => {
  const { root, list } = picking(() => {});
  await list.show(null, ["louis", "pro"]);
  assert.deepEqual(
    root.list.items.map((item) => item.textContent),
    ["louis · 5h 10%", "pro · 5h 10%"],
  );
});

test("a list that starts keeps its line to the form", async () => {
  const root = fakeRoot();
  const list = new AccountList(root, { labelsOf: labels, onStart: async () => {}, onForm: () => {} });
  await list.show({ directory: "/p", account: "" }, ["louis"]);
  assert.equal(root.list.items.length, 2);
  assert.equal(root.list.items[1].textContent, "other folder or name…");
});

test("with no project, the first account is picked", async () => {
  const { list } = picking(() => {});
  await list.show(null, ["louis", "pro"]);
  assert.equal(list.picked(), "louis");
});

test("in a project, its last account is picked", async () => {
  const { list } = picking(() => {});
  await list.show({ directory: "/p", account: "pro" }, ["louis", "pro"]);
  assert.equal(list.picked(), "pro");
});

test("the arrows stop on the last account of a list that only picks", async () => {
  const { root, list } = picking(() => {});
  await list.show(null, ["louis", "pro"]);
  key(root, "ArrowDown");
  key(root, "ArrowDown");
  assert.equal(list.picked(), "pro");
  assert.deepEqual(selectedText(root), ["pro · 5h 10%"]);
  key(root, "ArrowUp");
  key(root, "ArrowUp");
  assert.equal(list.picked(), "louis");
});

test("a click picks the account and starts nothing", async () => {
  let entered = 0;
  const { root, list } = picking(() => entered++);
  await list.show(null, ["louis", "pro"]);
  root.list.items[1].onclick();
  assert.equal(list.picked(), "pro");
  assert.deepEqual(selectedText(root), ["pro · 5h 10%"]);
  assert.equal(entered, 0);
});

test("Enter in a list that only picks sends what it is in", async () => {
  let entered = 0;
  const { root, list } = picking(() => entered++);
  await list.show(null, ["louis"]);
  key(root, "Enter");
  assert.equal(entered, 1);
});

test("the line picked is scrolled into sight", async () => {
  const { root, list } = picking(() => {});
  const accounts = Array.from({ length: 12 }, (_, index) => `account${index}`);
  await list.show(null, accounts);
  for (let step = 0; step < 11; step++) key(root, "ArrowDown");
  assert.equal(list.picked(), "account11");
  assert.equal(root.list.items[11].scrolledTo, true);
});

test("a list with no account picks nothing", async () => {
  const { list } = picking(() => {});
  await list.show(null, []);
  assert.equal(list.picked(), "");
});

test("the form shown again picks again from the first account", async () => {
  const { root, list } = picking(() => {});
  await list.show(null, ["louis", "pro"]);
  key(root, "ArrowDown");
  list.forget();
  await list.show(null, ["louis", "pro"]);
  assert.equal(list.picked(), "louis");
});
