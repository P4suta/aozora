import { readFileSync } from "node:fs";

import * as esbuild from "esbuild";

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
  target: "node20",
  outfile: "out/extension.js",
  external: ["vscode"],
  loader: { ".css": "text" },
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
