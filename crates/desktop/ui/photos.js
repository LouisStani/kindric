// The last screen seen of each session, shown while `claude attach` starts again (decision 338):
// it takes about 250 ms before Claude Code draws. No browser here, so `node --test` can check it
// (photos.test.js); panes.js takes and shows the photos.

// A few kilobytes each; the oldest goes past this many sessions.
export const KEPT = 32;

function keyOf(session) {
  return `${session.account}\n${session.short_id}`;
}

export class ScreenPhotos {
  constructor(limit = KEPT) {
    this.limit = limit;
    this.photos = new Map();
  }

  // `photo` is { text, columns }: the screen as terminal sequences, and how wide it was.
  keep(session, photo) {
    const key = keyOf(session);
    // Taken again, it counts as the newest.
    this.photos.delete(key);
    this.photos.set(key, photo);
    while (this.photos.size > this.limit) this.photos.delete(this.photos.keys().next().value);
  }

  get(session) {
    return this.photos.get(keyOf(session)) ?? null;
  }
}

// A photo taken at another width would wrap its lines until Claude Code draws again.
export function fits(photo, columns) {
  return photo !== null && photo.columns === columns && photo.text !== "";
}
