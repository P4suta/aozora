import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  AsyncGeneration,
  anyPositionInRange,
  canonicalizeSnapshotMatches,
  compareRangesDescending,
  documentVersionMatches,
  expandWrapText,
  finalCursorOffsets,
  findSlugAtOffset,
  htmlFileName,
} from "./extensionLogic.ts";
import { parseGaijiSpansResponse, parseRenderHtmlResult } from "./lspWire.ts";
import {
  AOZORA_WIRE_SCHEMA_VERSION,
  decodeLintSource,
  formatLintJson,
  formatLintProcessResult,
  WORKSPACE_LINT_SOURCE_LIMIT,
  workspaceLintSourceSizeSupported,
} from "./workspaceLintLogic.ts";

const here = dirname(fileURLToPath(import.meta.url));

test("slug lookup uses half-open offsets", () => {
  const line = "a［＃ぼうてん］b";
  const match = findSlugAtOffset(line, 2);
  assert.deepEqual(match, { start: 1, end: 8, body: "［＃ぼうてん］" });
  assert.equal(findSlugAtOffset(line, 8), undefined);
});

test("slug lookup accepts every supported half-width bracket form", () => {
  for (const body of [
    "［＃ぼうてん］",
    "［＃ぼうてん]",
    "［#ぼうてん］",
    "［#ぼうてん]",
    "[＃ぼうてん］",
    "[＃ぼうてん]",
    "[#ぼうてん］",
    "[#ぼうてん]",
  ]) {
    assert.equal(findSlugAtOffset(body, 0)?.body, body);
  }
});

test("canonicalization rejects a changed document snapshot", () => {
  assert.equal(canonicalizeSnapshotMatches(4, 4, "［＃ぼうてん］", "［＃ぼうてん］"), true);
  assert.equal(canonicalizeSnapshotMatches(4, 5, "［＃ぼうてん］", "［＃ぼうてん］"), false);
  assert.equal(canonicalizeSnapshotMatches(4, 4, "［＃ぼうてん］", "［＃傍点］"), false);
});

test("document versions gate asynchronous editor results", () => {
  assert.equal(documentVersionMatches(8, 8), true);
  assert.equal(documentVersionMatches(8, 9), false);
});

test("HTML export derives a portable filename from every supported source suffix", () => {
  assert.equal(htmlFileName("/books/蜘蛛の糸.aozora.txt"), "蜘蛛の糸.html");
  assert.equal(htmlFileName("C:\\books\\走れメロス.text"), "走れメロス.html");
  assert.equal(htmlFileName("Untitled-1"), "Untitled-1.html");
  assert.equal(htmlFileName(".txt"), "aozora.html");
});

test("ranges are ordered from the end of the document", () => {
  const ranges = [
    { start: { line: 0, character: 1 }, end: { line: 0, character: 2 } },
    { start: { line: 2, character: 0 }, end: { line: 2, character: 1 } },
    { start: { line: 0, character: 4 }, end: { line: 0, character: 5 } },
  ];
  ranges.sort(compareRangesDescending);
  assert.deepEqual(
    ranges.map((range) => [range.start.line, range.start.character]),
    [
      [2, 0],
      [0, 4],
      [0, 1],
    ],
  );
});

test("every cursor keeps a wrap tab stop after simultaneous edits", () => {
  const first = expandWrapText("｜BASE《$0》", "$0}");
  const second = expandWrapText("「BASE」$0", "後");
  assert.deepEqual(first, { text: "｜$0}《》", cursorOffset: 5 });
  assert.deepEqual(second, { text: "「後」", cursorOffset: 3 });
  assert.deepEqual(
    finalCursorOffsets([
      { start: 8, end: 9, ...second },
      { start: 1, end: 4, ...first },
    ]),
    [14, 6],
  );
});

test("a gaiji remains expanded for any cursor inside its half-open range", () => {
  const range = {
    start: { line: 2, character: 3 },
    end: { line: 2, character: 8 },
  };
  assert.equal(
    anyPositionInRange(
      [
        { line: 0, character: 0 },
        { line: 2, character: 4 },
      ],
      range,
    ),
    true,
  );
  assert.equal(anyPositionInRange([{ line: 2, character: 8 }], range), false);
});

test("only the latest live asynchronous generation can commit", async () => {
  const generation = new AsyncGeneration();
  const commits = [];
  const firstResult = Promise.withResolvers();
  const secondResult = Promise.withResolvers();
  const first = generation.begin();
  const firstApply = firstResult.promise.then((value) => {
    if (generation.isCurrent(first)) {
      commits.push(value);
    }
  });
  const second = generation.begin();
  const secondApply = secondResult.promise.then((value) => {
    if (generation.isCurrent(second)) {
      commits.push(value);
    }
  });
  secondResult.resolve("new");
  await secondApply;
  firstResult.resolve("old");
  await firstApply;
  assert.deepEqual(commits, ["new"]);
  generation.dispose();
  assert.equal(generation.isCurrent(second), false);
});

test("custom render responses are checked at the LSP boundary", () => {
  assert.deepEqual(parseRenderHtmlResult({ html: "<p>青空</p>", paused: false }), {
    html: "<p>青空</p>",
    paused: false,
  });
  for (const malformed of [
    null,
    [],
    {},
    { html: 7, paused: false },
    { html: "<p />", paused: "false" },
  ]) {
    assert.throws(() => parseRenderHtmlResult(malformed), /invalid aozora\/renderHtml response/);
  }
});

test("custom gaiji responses reject malformed UTF-16 ranges", () => {
  const valid = {
    spans: [
      {
        range: {
          start: { line: 1, character: 2 },
          end: { line: 1, character: 9 },
        },
        resolved: "か\u3099",
      },
    ],
  };
  assert.deepEqual(parseGaijiSpansResponse(valid), valid);

  for (const malformed of [
    null,
    { spans: {} },
    { spans: [{ ...valid.spans[0], resolved: 7 }] },
    { spans: [{ ...valid.spans[0], resolved: "" }] },
    {
      spans: [
        {
          ...valid.spans[0],
          range: { start: { line: -1, character: 0 }, end: { line: 0, character: 0 } },
        },
      ],
    },
    {
      spans: [
        {
          ...valid.spans[0],
          range: { start: { line: 1, character: 9 }, end: { line: 1, character: 2 } },
        },
      ],
    },
  ]) {
    assert.throws(() => parseGaijiSpansResponse(malformed), /invalid aozora\/gaijiSpans response/);
  }
});

test("workspace lint maps JSON byte spans to matcher locations", () => {
  const output = JSON.stringify({
    schemaVersion: AOZORA_WIRE_SCHEMA_VERSION,
    data: [
      {
        kind: "non_canonical_directive",
        severity: "warning",
        source: "source",
        span: { start: 9, end: 33 },
      },
    ],
  });
  assert.deepEqual(formatLintJson(output, "/tmp/source.txt", "前段。［＃改行を挿入］"), [
    "/tmp/source.txt:1:4: warning[non_canonical_directive]: non canonical directive",
  ]);
});

test("workspace lint keeps a UTF-8 BOM in the CLI byte coordinate space", () => {
  const source = decodeLintSource(
    Uint8Array.from([0xef, 0xbb, 0xbf, ...new TextEncoder().encode("あ［＃字下げ終わり］")]),
  );
  const output = JSON.stringify({
    schemaVersion: AOZORA_WIRE_SCHEMA_VERSION,
    data: [
      {
        kind: "non_canonical_directive",
        severity: "warning",
        source: "source",
        span: { start: 6, end: 33 },
      },
    ],
  });
  assert.deepEqual(formatLintJson(output, "/tmp/source.txt", source), [
    "/tmp/source.txt:1:2: warning[non_canonical_directive]: non canonical directive",
  ]);
});

test("workspace lint rejects bytes the CLI cannot decode", () => {
  assert.throws(() => decodeLintSource(Uint8Array.from([0x82])));
});

test("workspace lint rejects files that exceed the editor source budget", () => {
  assert.equal(workspaceLintSourceSizeSupported(WORKSPACE_LINT_SOURCE_LIMIT), true);
  assert.equal(workspaceLintSourceSizeSupported(WORKSPACE_LINT_SOURCE_LIMIT + 1), false);
  assert.equal(workspaceLintSourceSizeSupported(-1), false);
  assert.equal(workspaceLintSourceSizeSupported(Number.NaN), false);
});

test("workspace lint consumes diagnostics from stderr and rejects stdout", () => {
  const stderr = JSON.stringify({
    schemaVersion: AOZORA_WIRE_SCHEMA_VERSION,
    data: [
      {
        kind: "non_canonical_directive",
        severity: "warning",
        source: "source",
        span: { start: 0, end: 3 },
      },
    ],
  });
  assert.deepEqual(
    formatLintProcessResult({ code: 0, stdout: "", stderr }, "/tmp/source.txt", "青空"),
    {
      lines: ["/tmp/source.txt:1:1: warning[non_canonical_directive]: non canonical directive"],
      failed: false,
    },
  );
  assert.throws(
    () =>
      formatLintProcessResult({ code: 0, stdout: stderr, stderr: "" }, "/tmp/source.txt", "青空"),
    /unexpected output to stdout/,
  );
});

test("workspace lint rejects an unsupported diagnostic schema", () => {
  assert.throws(
    () =>
      formatLintProcessResult(
        {
          code: 0,
          stdout: "",
          stderr: JSON.stringify({ schemaVersion: AOZORA_WIRE_SCHEMA_VERSION + 1, data: [] }),
        },
        "/tmp/source.txt",
        "",
      ),
    /invalid diagnostic JSON/,
  );
});

test("workspace lint schema stays aligned with the generated wire types", () => {
  const generatedTypes = readFileSync(
    join(here, "..", "..", "..", "crates", "aozora-wasm", "types", "aozora_types.d.ts"),
    "utf8",
  );
  const schemaVersion = /schemaVersion: (\d+);/.exec(generatedTypes)?.[1];
  assert.equal(Number(schemaVersion), AOZORA_WIRE_SCHEMA_VERSION);
});

test("workspace lint rejects malformed diagnostic fields and byte spans", () => {
  const diagnostic = {
    kind: "non_canonical_directive",
    severity: "warning",
    source: "source",
    span: { start: 0, end: 3 },
  };
  const output = (value) =>
    JSON.stringify({
      schemaVersion: AOZORA_WIRE_SCHEMA_VERSION,
      data: [value],
    });
  for (const malformed of [
    { ...diagnostic, severity: "fatal" },
    { ...diagnostic, severity: "info" },
    { ...diagnostic, source: "rendered" },
    { ...diagnostic, span: { start: 1, end: 3 } },
    { ...diagnostic, span: { start: 0, end: 999 } },
    { ...diagnostic, span: { start: 3, end: 0 } },
  ]) {
    assert.throws(() => formatLintJson(output(malformed), "/tmp/source.txt", "青空"));
  }
});

test("workspace lint maps wire notes to problem-matcher info severity", () => {
  const output = JSON.stringify({
    schemaVersion: AOZORA_WIRE_SCHEMA_VERSION,
    data: [
      {
        kind: "context",
        severity: "note",
        source: "source",
        span: { start: 0, end: 0 },
      },
    ],
  });
  assert.deepEqual(formatLintJson(output, "/tmp/source.txt", ""), [
    "/tmp/source.txt:1:1: info[context]: context",
  ]);
});

test("workspace lint preserves valid internal diagnostics", () => {
  const output = JSON.stringify({
    schemaVersion: AOZORA_WIRE_SCHEMA_VERSION,
    data: [
      {
        kind: "registry_position_mismatch",
        severity: "error",
        source: "internal",
        span: { start: 0, end: 0 },
      },
    ],
  });
  assert.deepEqual(formatLintJson(output, "/tmp/source.txt", ""), [
    "/tmp/source.txt:1:1: error[registry_position_mismatch]: registry position mismatch",
  ]);
});

test("workspace lint uses argv child processes and a matching JSON contract", () => {
  const canonicalizeSource = readFileSync(join(here, "canonicalize.ts"), "utf8");
  const commandSource = readFileSync(join(here, "cliCommands.ts"), "utf8");
  const extensionSource = readFileSync(join(here, "extension.ts"), "utf8");
  const terminalSource = readFileSync(join(here, "workspaceLint.ts"), "utf8");
  assert.match(commandSource, /new vscode\.CustomExecution\(/);
  assert.match(commandSource, /workspace\.findFiles\(/);
  assert.match(terminalSource, /"lint",\s+"--encoding",\s+"auto",\s+"--format",\s+"json"/);
  assert.match(terminalSource, /shell: false/);
  assert.doesNotMatch(commandSource, /sendText\(/);
  assert.doesNotMatch(commandSource, /new vscode\.ProcessExecution\(/);
  assert.match(extensionSource, /registerCliCommands\(context, client, defaultBinary\)/);
  assert.doesNotMatch(extensionSource, /registerCliCommands\(context, client, lspPath\)/);
  assert.match(canonicalizeSource, /version,\s*range:/);

  const packageJson = JSON.parse(readFileSync(join(here, "..", "package.json"), "utf8"));
  const matcher = packageJson.contributes.problemMatchers.find(
    (candidate) => candidate.name === "aozora-lint",
  );
  assert.ok(matcher);
  const match = new RegExp(matcher.pattern.regexp).exec(
    "/tmp/source.txt:1:4: warning[non_canonical_directive]: non canonical directive",
  );
  assert.deepEqual(match?.slice(1), [
    "/tmp/source.txt",
    "1",
    "4",
    "warning",
    "non_canonical_directive",
    "non canonical directive",
  ]);
});

test("preview and gaiji rendering discard stale asynchronous results", () => {
  const preview = readFileSync(join(here, "preview.ts"), "utf8");
  const gaiji = readFileSync(join(here, "gaijiFold.ts"), "utf8");
  assert.match(preview, /generation\.isCurrent\(generation\)/);
  assert.match(preview, /generation\.invalidate\(\)/);
  assert.match(preview, /generation\.dispose\(\)/);
  assert.match(gaiji, /refresh\.isCurrent\(generation\)/);
  assert.match(gaiji, /document\.version !== version/);
  assert.match(gaiji, /window\.visibleTextEditors/);
  assert.match(gaiji, /spansFitDocument\(document, response\.spans\)/);
  assert.match(gaiji, /this\.clear\(editor\)/);
});

test("asynchronous editor commands refuse stale document state", () => {
  const commands = readFileSync(join(here, "cliCommands.ts"), "utf8");
  const outline = readFileSync(join(here, "outline.ts"), "utf8");
  const snippets = readFileSync(join(here, "snippetTrigger.ts"), "utf8");
  const wraps = readFileSync(join(here, "wrap.ts"), "utf8");
  assert.equal(commands.match(/documentVersionMatches\(version, document\.version\)/g)?.length, 2);
  assert.equal(outline.match(/documentVersionMatches\(version, document\.version\)/g)?.length, 3);
  assert.match(snippets, /window\.activeTextEditor === editor/);
  assert.equal(snippets.match(/documentVersionMatches\(version \+ 1, doc\.version\)/g)?.length, 2);
  assert.match(wraps, /documentVersionMatches\(version \+ 1, document\.version\)/);
});

test("plaintext detection reads only the bounded document prefix", () => {
  const extension = readFileSync(join(here, "extension.ts"), "utf8");
  assert.match(extension, /document\.getText\(\s*new Range\(/);
  assert.doesNotMatch(extension, /document\.getText\(\)\.slice/);
});
