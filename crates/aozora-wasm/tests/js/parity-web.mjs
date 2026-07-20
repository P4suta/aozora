import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { isDeepStrictEqual } from "node:util";

const [packageArgument, fixturesArgument, schemaArgument] = process.argv.slice(2);
if (!packageArgument || !fixturesArgument || !schemaArgument) {
  console.error("usage: parity-web.mjs <package> <fixtures> <schema-version>");
  process.exit(2);
}

const packageRoot = resolve(packageArgument);
const fixturesRoot = resolve(fixturesArgument);
const api = await import(pathToFileURL(join(packageRoot, "aozora_wasm.js")));
const bytes = readFileSync(join(packageRoot, "aozora_wasm_bg.wasm"));
await api.default({ module_or_path: bytes });
if (api.schemaVersion() !== Number(schemaArgument)) {
  throw new Error("wire schema version mismatch");
}

const textSurfaces = [
  ["expected.html", (document) => document.toHtml()],
  ["expected.serialize.txt", (document) => document.toSource()],
];
const structuredSurfaces = [
  ["expected.diagnostics.json", (document) => document.diagnostics()],
  ["expected.nodes.json", (document) => document.nodes()],
  ["expected.pairs.json", (document) => document.pairs()],
  ["expected.container_pairs.json", (document) => document.containerPairs()],
];
const directories = readdirSync(fixturesRoot)
  .filter((name) => statSync(join(fixturesRoot, name)).isDirectory())
  .sort();
if (directories.length === 0) {
  throw new Error(`no fixtures under ${fixturesRoot}`);
}

let checks = 0;
for (const name of directories) {
  const directory = join(fixturesRoot, name);
  const document = new api.Document(readFileSync(join(directory, "source.txt"), "utf8"));
  for (const [file, accessor] of textSurfaces) {
    if (accessor(document) !== readFileSync(join(directory, file), "utf8")) {
      throw new Error(`${name}/${file} drift`);
    }
    checks += 1;
  }
  for (const [file, accessor] of structuredSurfaces) {
    const expected = JSON.parse(readFileSync(join(directory, file), "utf8")).data;
    if (!isDeepStrictEqual(accessor(document), expected)) {
      throw new Error(`${name}/${file} drift`);
    }
    checks += 1;
  }
  document.free();
}

const edited = new api.Document("｜青空《あおぞら》");
edited.edit([{ start: 3, end: 9, replacement: "蒼空" }]);
if (!edited.toSource().includes("蒼空")) {
  throw new Error("installed npm artifact did not apply an edit");
}
edited.free();
console.log(`parity-web: ${checks} checks passed`);
