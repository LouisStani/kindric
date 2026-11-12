// Themes: values for the color variables of style.css, names without "--". Ember is the one
// style.css starts with; "custom" is ember with the colors of theme.json over it.

export const THEMES = {
  ember: {
    background: "#1a0e06",
    surface: "#24150b",
    raised: "#2e1a0c",
    line: "#3a2412",
    "line-strong": "#5c300f",
    text: "#f6e7d4",
    "text-strong": "#fff6ea",
    muted: "#d9bfa0",
    faint: "#8a5a30",
    terminal: "#1f120a",
    "terminal-text": "#f6e7d4",
    selection: "#5c300f",
    active: "#aa601f",
    "on-active": "#f6e7d4",
    waiting: "#f2b544",
    working: "#f6e7d4",
    done: "#a8c66c",
    error: "#e8735a",
    "on-accent": "#1a0e06",
  },
  paper: {
    background: "#f7efe4",
    surface: "#f0e5d6",
    raised: "#ece0cf",
    line: "#ddcbb3",
    "line-strong": "#c09a70",
    text: "#2b1a0e",
    "text-strong": "#1a0e06",
    muted: "#5c432c",
    faint: "#7d5c3c",
    terminal: "#fbf6ee",
    "terminal-text": "#2b1a0e",
    selection: "#ead2b0",
    active: "#9a5418",
    "on-active": "#fff8ef",
    waiting: "#8f560b",
    working: "#2b1a0e",
    done: "#4f6f22",
    error: "#b03c25",
    "on-accent": "#fff8ef",
  },
  graphite: {
    background: "#121314",
    surface: "#1a1b1d",
    raised: "#222326",
    line: "#2e3034",
    "line-strong": "#4a4d52",
    text: "#e6e6e6",
    "text-strong": "#ffffff",
    muted: "#b4b7bc",
    faint: "#7c8087",
    terminal: "#161719",
    "terminal-text": "#e6e6e6",
    selection: "#3a3d42",
    active: "#3f6fb8",
    "on-active": "#ffffff",
    waiting: "#e5b84b",
    working: "#e6e6e6",
    done: "#8cc06a",
    error: "#e57260",
    "on-accent": "#121314",
  },
};

const ORDER = ["ember", "paper", "graphite", "custom"];

// Light themes tell the engine, so form controls and scroll bars follow.
const LIGHT = new Set(["paper"]);

export function applyTheme(name, colors) {
  const root = document.documentElement;
  for (const [color, value] of Object.entries(colors)) root.style.setProperty(`--${color}`, value);
  root.style.colorScheme = LIGHT.has(name) ? "light" : "dark";
}

// The theme after `current`; "custom" only when theme.json exists.
export function nextTheme(current, hasCustom) {
  const names = hasCustom ? ORDER : ORDER.slice(0, -1);
  return names[(names.indexOf(current) + 1) % names.length];
}

// The theme of a project after `current`, null standing for kindric's own, which comes last.
export function nextProjectTheme(current, hasCustom) {
  const names = [...(hasCustom ? ORDER : ORDER.slice(0, -1)), null];
  return names[(names.indexOf(current) + 1) % names.length];
}
