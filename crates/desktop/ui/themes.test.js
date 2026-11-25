// Tests of the themes: `node --test crates/desktop/ui/themes.test.js`.
// Contrast is the WCAG 2 ratio (https://www.w3.org/TR/WCAG21/#dfn-contrast-ratio); 4.5 is the
// floor it sets for text of normal size.

import { test } from "node:test";
import assert from "node:assert/strict";
import { customColorsOf, THEMES } from "./themes.js";

function luminance(hex) {
  const channels = [1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16) / 255);
  const [r, g, b] = channels.map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a, b) {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (light + 0.05) / (dark + 0.05);
}

for (const [name, colors] of Object.entries(THEMES)) {
  test(`${name}: faint text reads on every ground`, () => {
    for (const ground of ["background", "surface", "raised", "terminal"]) {
      const ratio = contrast(colors["faint-text"], colors[ground]);
      assert.ok(ratio >= 4.5, `faint-text on ${ground}: ${ratio.toFixed(2)}`);
    }
  });

  test(`${name}: text on the active color reads`, () => {
    const ratio = contrast(colors["on-active"], colors.active);
    assert.ok(ratio >= 4.5, `on-active on active: ${ratio.toFixed(2)}`);
  });
}

test("the contrast of black on white is 21", () => {
  assert.equal(contrast("#000000", "#ffffff"), 21);
});

test("a theme.json without faint-text keeps its faint for text", () => {
  assert.equal(customColorsOf({ faint: "#123456" })["faint-text"], "#123456");
  assert.equal(customColorsOf({ faint: "#123456", "faint-text": "#abcdef" })["faint-text"], "#abcdef");
  assert.equal(customColorsOf({})["faint-text"], THEMES.ember["faint-text"]);
});
