// Timings of the page, written by Rust to trace.log when the app runs with
// KINDRIC_TRACE (see trace.rs). Off, nothing is measured or sent.

const { invoke } = window.__TAURI__.core;

let enabled = false;

export async function startTrace() {
  enabled = (await invoke("trace_enabled").catch(() => false)) === true;
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
