// VS Code extension bundler.
//
// esbuild collapses `src/extension.ts` + every TypeScript module under
// `src/` + every npm runtime dependency (`vscode-languageclient` and
// its transitive deps — minimatch / semver / vscode-jsonrpc /
// vscode-languageserver-protocol / vscode-languageserver-types) into
// a *single* CommonJS file at `out/extension.js`.
//
// Why this matters: the `.vsix` no longer needs to ship `node_modules/`
// at install time. The previous tsc-emit pipeline left runtime
// `require('vscode-languageclient/node')` lookups in the compiled
// output that VS Code's extension host could not resolve once the
// .vsix unpacked into `~/.vscode/extensions/<id>/` (no install step
// runs there). The classic "Cannot find module 'vscode-languageclient/
// node'" failure on first activation is the symptom.
//
// Why CommonJS, why `external: ['vscode']`: VS Code's extension host
// is `require()`-based and provides the `vscode` API as a built-in
// module that the bundler must NOT try to resolve from the file
// system. https://code.visualstudio.com/api/working-with-extensions/bundling-extension

import { readFileSync } from "node:fs";

import * as esbuild from "esbuild";

// The bundle target has to match the Node the extension host actually
// runs: VS Code 1.91 ships Electron 29.4.0 / Node.js 20.9.0. Read the
// engine back from package.json instead of restating it in a comment —
// raising `engines.vscode` now fails the build until someone re-derives
// the target from the new engine's Electron/Node pair.
const TARGET_ENGINE = "^1.91.0";
const { engines } = JSON.parse(readFileSync(new URL("package.json", import.meta.url), "utf8"));
if (engines.vscode !== TARGET_ENGINE) {
  throw new Error(
    `engines.vscode is ${engines.vscode}, but the esbuild target assumes ${TARGET_ENGINE} ` +
      "(Node 20). Re-derive `target` from the Electron/Node pair the new engine ships.",
  );
}

const production = process.argv.includes("--production");
const watch = process.argv.includes("--watch");

const baseConfig = {
  entryPoints: ["src/extension.ts"],
  bundle: true,
  format: "cjs",
  platform: "node",
  // Emit syntax the host can parse without polyfilling — see the
  // engine check above for where this comes from.
  target: "node20",
  outfile: "out/extension.js",
  // `vscode` is injected by the host. Bundling it would either fail
  // (the npm package doesn't actually contain the runtime — only
  // typings) or smuggle in a duplicate that diverges from the host's
  // version.
  external: ["vscode"],
  // Inline the renderer's canonical notation stylesheet
  // (crates/aozora-render/assets/aozora-notation.css) as a string at
  // build time, so the preview + HTML export share one source of truth
  // instead of hand-rolling `.aozora-*` CSS (which had drifted to dead
  // class names). The `.vsix` ships the inlined copy — no asset file.
  loader: { ".css": "text" },
  // Production: minify for size, no source map (those leak source via
  // .vsix and inflate the package without runtime benefit). Dev: keep
  // source map for breakpoints in the Extension Development Host.
  minify: production,
  sourcemap: !production,
  sourcesContent: false,
  logLevel: production ? "warning" : "info",
};

if (watch) {
  const ctx = await esbuild.context(baseConfig);
  await ctx.watch();
  // eslint-disable-next-line no-console
  console.log("esbuild: watching src/ for changes…");
} else {
  await esbuild.build(baseConfig);
}
