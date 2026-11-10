// Timings of the page, written by Rust to trace.log when the app runs with
// KINDRIC_TRACE (see trace.rs). Off, nothing is measured or sent.

const { invoke } = window.__TAURI__.core;

let enabled = false;

// A frame drawn this much after the one before is a jerk the eye sees.
const LONG_FRAME_MS = 50;

export async function startTrace() {
  enabled = (await invoke("trace_enabled").catch(() => false)) === true;
  if (enabled) watchFrames();
}

// Records every long frame while the window is shown: when the page stutters, and how badly.
function watchFrames() {
  let last = performance.now();
  const frame = (now) => {
    const gap = now - last;
    last = now;
    if (gap > LONG_FRAME_MS && document.visibilityState === "visible") {
      invoke("trace", { what: "long frame", ms: gap }).catch(() => {});
    }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

// A fact without a duration: the size a pane measured, the one it sent.
export function note(what) {
  if (enabled) invoke("trace", { what, ms: 0 }).catch(() => {});
}

// `start` comes from performance.now().
export function mark(what, start) {
  if (enabled) invoke("trace", { what, ms: performance.now() - start }).catch(() => {});
}

function nextFrame() {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

// Until `run` is done and the next frame is drawn: what the user waits for.
export async function traced(what, run) {
  if (!enabled) return run();
  const start = performance.now();
  try {
    return await run();
  } finally {
    await nextFrame();
    mark(what, start);
  }
}
