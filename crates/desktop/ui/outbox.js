// What Rust keeps for the page (outbox.rs): terminal output and the list of sessions, taken with
// the command `take_outbox` whenever Rust rings. WebKitGTK keeps memory for every script of new
// text it runs, and Tauri sends each channel message and each event as one: the ring is always
// the same script, and what waits comes back as the answer to a command.

const OUTPUT = 0;
const SESSIONS = 1;

// The letters of `taken` as Rust writes them (`Outbox::take`): `{ output, bytes }` for a
// terminal's output, `{ sessions }` for the list. Tauri answers with an array of numbers instead
// of an ArrayBuffer when it could not fetch the command.
export function letters(taken) {
  const buffer = taken instanceof ArrayBuffer ? taken : Uint8Array.from(taken).buffer;
  const data = new DataView(buffer);
  const found = [];
  let at = 0;
  while (at < buffer.byteLength) {
    const kind = data.getUint8(at);
    at += 1;
    if (kind === OUTPUT) {
      const length = data.getUint32(at + 4, true);
      found.push({ output: data.getUint32(at, true), bytes: new Uint8Array(buffer, at + 8, length) });
      at += 8 + length;
    } else if (kind === SESSIONS) {
      const length = data.getUint32(at, true);
      found.push({ sessions: JSON.parse(new TextDecoder().decode(new Uint8Array(buffer, at + 4, length))) });
      at += 4 + length;
    } else throw new Error(`unknown letter ${kind} in the outbox`);
  }
  return found;
}

// Hands each terminal its output, and the list to `onSessions`. `take()` resolves to what waits.
export class Outbox {
  constructor(take, onSessions, retryAfter = 1000) {
    this.take = take;
    this.onSessions = onSessions;
    this.retryAfter = retryAfter;
    this.terminals = new Map();
    this.next = 1;
    this.taking = false;
    this.again = false;
  }

  // Numbers a terminal whose output goes to `write(bytes)`: Rust is given that number.
  open(write) {
    const number = this.next;
    this.next += 1;
    this.terminals.set(number, write);
    return number;
  }

  close(number) {
    this.terminals.delete(number);
  }

  // Takes what waits. Called again meanwhile, it takes once more after. Rust rings only once until
  // it is taken from, so a failed take is tried again.
  async collect() {
    if (this.taking) {
      this.again = true;
      return;
    }
    this.taking = true;
    try {
      do {
        this.again = false;
        for (const letter of letters(await this.take())) this.deliver(letter);
      } while (this.again);
    } catch (error) {
      console.error("outbox", error);
      setTimeout(() => this.collect(), this.retryAfter);
    } finally {
      this.taking = false;
    }
  }

  deliver(letter) {
    try {
      if ("sessions" in letter) this.onSessions(letter.sessions);
      else this.terminals.get(letter.output)?.(letter.bytes);
    } catch (error) {
      console.error("outbox", error);
    }
  }
}
