// Cross-surface parity gate — wasm (Node) channel.
//
// One golden authority (crates/aozora-conformance/fixtures/render), N thin
// walkers. This walker loads the wasm-pack `--target nodejs` build and
// asserts text outputs byte-for-byte and typed projections semantically
// against the same golden data the Rust gate pins.
//
// Usage:  node parity.mjs <path-to-wasm-nodejs-pkg>
// Driven by `just parity-wasm` locally and by ci.yml.
// `wasm-build` job (host runner).

import { createRequire } from "node:module";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";
import { isDeepStrictEqual } from "node:util";

const here = dirname(fileURLToPath(import.meta.url));

const pkgArg = process.argv[2];
if (!pkgArg) {
  console.error("usage: node parity.mjs <path-to-wasm-nodejs-pkg>");
  process.exit(2);
}

// wasm-pack `--target nodejs` emits a CommonJS module that instantiates
// the wasm synchronously at require-time — no async init needed.
const require = createRequire(import.meta.url);
const { Document } = require(resolve(pkgArg));

// parity.mjs lives at crates/aozora-wasm/tests/js/ — three levels up is
// crates/, then the shared golden corpus.
const fixturesRoot = resolve(
  here,
  "..",
  "..",
  "..",
  "aozora-conformance",
  "fixtures",
  "render",
);

const TEXT_SURFACES = [
  ["expected.html", (d) => d.toHtml()],
  ["expected.serialize.txt", (d) => d.toSource()],
];

const STRUCTURED_SURFACES = [
  ["expected.diagnostics.json", (d) => d.diagnostics()],
  ["expected.nodes.json", (d) => d.nodes()],
  ["expected.pairs.json", (d) => d.pairs()],
  ["expected.container_pairs.json", (d) => d.containerPairs()],
];

const dirs = readdirSync(fixturesRoot)
  .filter((name) => statSync(join(fixturesRoot, name)).isDirectory())
  .sort();

if (dirs.length === 0) {
  console.error(`parity-wasm: no fixtures under ${fixturesRoot}`);
  process.exit(1);
}

let checks = 0;
let failures = 0;

for (const name of dirs) {
  const fdir = join(fixturesRoot, name);
  const source = readFileSync(join(fdir, "source.txt"), "utf8");
  const doc = new Document(source);
  for (const [file, accessor] of TEXT_SURFACES) {
    const golden = readFileSync(join(fdir, file), "utf8");
    const actual = accessor(doc);
    checks += 1;
    if (actual !== golden) {
      failures += 1;
      console.error(`DRIFT ${name}/${file}`);
      console.error(`  golden: ${JSON.stringify(golden.slice(0, 160))}`);
      console.error(`  actual: ${JSON.stringify(actual.slice(0, 160))}`);
    }
  }
  for (const [file, accessor] of STRUCTURED_SURFACES) {
    const golden = JSON.parse(readFileSync(join(fdir, file), "utf8")).data;
    const actual = accessor(doc);
    checks += 1;
    if (!isDeepStrictEqual(actual, golden)) {
      failures += 1;
      console.error(`DRIFT ${name}/${file}`);
      console.error(`  golden: ${JSON.stringify(golden).slice(0, 320)}`);
      console.error(`  actual: ${JSON.stringify(actual).slice(0, 320)}`);
    }
  }
  // Free the wasm-side handle eagerly (nodejs target still exposes free()).
  if (typeof doc.free === "function") doc.free();
}

console.log(
  `parity-wasm: ${dirs.length} fixtures × ${TEXT_SURFACES.length + STRUCTURED_SURFACES.length} surfaces = ${checks} checks, ${failures} drift`,
);
process.exit(failures === 0 ? 0 : 1);
