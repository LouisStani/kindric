// What a session's dot shows in the bar and on its tab: its color, its shape, and the sign
// in it. Pure, so that node tests it.

// The state a chip shows. A session of another terminal cannot be opened here, whatever it
// does. One that waits for an answer (`asks`, set by Rust) shows it before a finished turn.
export function chipState(session) {
  if (!session.short_id) return "elsewhere";
  if (session.asks) return "asking";
  if (session.finished) return "finished";
  return session.state;
}

// The sign inside a dot: a question, a finished turn, or none (the dot's color says it).
export function dotSign(state) {
  if (state === "asking") return "?";
  if (state === "finished") return "✓";
  return "";
}

// The dot of a tab, from its most urgent session (Rust sorts them): only when that one
// needs the user, and a question first of all.
export function tabMark(session) {
  if (!session) return null;
  const state = chipState(session);
  if (state === "asking") return state;
  if (session.unread > 0) return "waiting";
  return state === "finished" || state === "waiting" ? state : null;
}

// The state the tooltip of a chip names.
export function stateWords(session) {
  if (session.asks) return session.waiting_for ? `asks you: ${session.waiting_for}` : "asks you";
  return session.finished ? "done" : session.state;
}
