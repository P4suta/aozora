// Regression guard for the preview webview's defense-in-depth against
// HTML injection (the `aozora/renderHtml` -> webview path).
//
// A full render test needs the VS Code Electron harness; until that
// exists, assert the security-critical configuration stays present in
// `preview.ts` so it can never be silently weakened. Plain ESM + the
// built-in `node:test` runner — no extra tooling, no types, and tsc
// skips `.mjs`. Run with `node --test` (or `bun run test`).
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
