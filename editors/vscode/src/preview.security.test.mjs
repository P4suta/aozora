import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const previewSrc = readFileSync(join(here, "preview.ts"), "utf8");

test("preview webview keeps scripts disabled", () => {
  assert.match(previewSrc, /enableScripts:\s*false/);
  assert.doesNotMatch(previewSrc, /enableScripts:\s*true/);
});

test("preview webview ships a strict Content-Security-Policy", () => {
  assert.match(previewSrc, /Content-Security-Policy/);
  assert.match(previewSrc, /default-src 'none'/);
});

test("preview webview locks local resource roots to nothing", () => {
  assert.match(previewSrc, /localResourceRoots:\s*\[\]/);
});
