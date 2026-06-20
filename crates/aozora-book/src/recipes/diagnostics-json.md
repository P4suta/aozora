# Diagnostics as JSON

**Problem.** You want the parser's diagnostics as a stable,
machine-readable JSON document — to feed an editor, a CI annotation,
or a cross-language tool.

## Solution (library)

The parser always produces a tree, even from malformed input;
diagnostics ride alongside it. `Tree::diagnostics` is the typed
slice, and `aozora::json::diagnostics` projects that slice
into the shared [wire envelope](../wire/overview.md) — the exact JSON
every binding (FFI, wasm, Python, Extism) emits.

```rust
use aozora::Document;
use aozora::json::diagnostics;

fn main() {
    // U+E001 is a private-use sentinel the parser reserves; feeding one
    // in raises a diagnostic without aborting the parse.
    let doc = Document::new("abc\u{E001}def");
    let tree = doc.parse();

    let json = diagnostics(tree.diagnostics());
    println!("{json}");
}
```

> The `wire` module is behind the `wire` Cargo feature on `aozora`.

## Expected output

```json
{"schemaVersion":1,"data":[{"kind":"source_contains_pua","severity":"warning","source":"source","span":{"start":3,"end":6},"codepoint":""}]}
```

Each entry is `{ kind, severity, source, span: { start, end },
codepoint? }`. `schema_version` lets a consumer branch before an added
variant shows up; see the [Wire format](../wire/overview.md) chapter
for the full schema and the `"unknown"` fallback contract.

## Walking diagnostics without serialising

If you are staying in Rust, you usually do not need JSON at all — read
the typed slice directly:

```rust
for d in tree.diagnostics() {
    // `Diagnostic` is an enum: `{d}` is the human message (thiserror),
    // `code()` the stable id, `span()` the byte range.
    let span = d.span();
    eprintln!("[{}] {d} @ {}..{}", d.code(), span.start, span.end);
}
```

Diagnostics are **non-fatal by design**: callers that want strict
behaviour treat any diagnostic as an error themselves. The
[Diagnostics catalogue](../notation/diagnostics.md) lists every stable
code.

## Solution (CLI)

For shell / CI use, `aozora check` lexes a file and reports
diagnostics, exiting non-zero under `--strict`:

```sh
aozora check src.txt              # human-readable; exit 0 even with warnings
aozora check --strict src.txt     # warnings → exit 1 (the CI gate)
cat src.txt | aozora check        # reads stdin
```

Pass `--diagnostic-format json` to get the exact `diagnostics`
envelope shown above — byte-identical to the library path and to what
every binding emits — straight from the shell, no Rust required:

```sh
aozora check --diagnostic-format json src.txt
```

Diagnostics print to **stderr** (where `json` is already the default
once stderr is piped), so redirect that stream to feed a tool like
`jq`. A clean file prints nothing and exits `0`:

```sh
aozora check --diagnostic-format json src.txt 2>&1 >/dev/null | jq .
```

See the [CLI reference](../ref/cli.md) for the full flag list and the
exit-code table.

## See also

- Runnable example: **`just example diagnostics`**
  (`crates/aozora/examples/diagnostics.rs`).
- [Diagnostics catalogue](../notation/diagnostics.md) — every code,
  severity, and what triggers it.
- [Wire format](../wire/overview.md) — the envelope schema and version
  contract.
- [CLI reference → `aozora check`](../ref/cli.md) — flags and exit
  codes.
