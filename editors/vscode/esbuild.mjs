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

import * as esbuild from "esbuild";

const production = process.argv.includes("--production");
const watch = process.argv.includes("--watch");

const baseConfig = {
  entryPoints: ["src/extension.ts"],
  bundle: true,
  format: "cjs",
  platform: "node",
  // VS Code 1.80 ships Node 18; use the same target so esbuild emits
  // syntax the host can parse without polyfilling. `engines.vscode`
  // in package.json is `^1.80`, anything older isn't supported.
  target: "node18",
  outfile: "out/extension.js",
  // `vscode` is injected by the host. Bundling it would either fail
  // (the npm package doesn't actually contain the runtime — only
  // typings) or smuggle in a duplicate that diverges from the host's
  // version.
  external: ["vscode"],
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
