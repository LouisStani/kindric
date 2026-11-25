// The messages kindric types into sessions no pane shows. Rust holds the session's lock meanwhile,
// about 2.6 s: opening it then would be refused as if another kindric window showed it.

function idOf(key) {
  return `${key.account}/${key.short_id}`;
}

export class Typing {
  constructor() {
    // The last message of each session being typed or waiting its turn.
    this.last = new Map();
  }

  busy(key) {
    return this.last.has(idOf(key));
  }

  // Runs `type` once the messages before it for the same session are over, failed or not.
  async during(key, type) {
    const id = idOf(key);
    const before = this.last.get(id) ?? Promise.resolve();
    const turn = before.catch(() => {}).then(type);
    this.last.set(id, turn);
    try {
      return await turn;
    } finally {
      if (this.last.get(id) === turn) this.last.delete(id);
    }
  }

  // Once nothing is being typed into the session any more.
  async over(key) {
    while (this.busy(key)) await this.last.get(idOf(key)).catch(() => {});
  }
}
