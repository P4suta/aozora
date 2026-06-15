# Diagnostics as JSON

**Problem.** You want the parser's diagnostics as a stable,
machine-readable JSON document — to feed an editor, a CI annotation,
or a cross-language tool.

## Solution (library)

The parser always produces a tree, even from malformed input;
diagnostics ride alongside it. `AozoraTree::diagnostics` is the typed
slice, and `aozora::wire::serialize_diagnostics` projects that slice
into the shared [wire envelope](../wire/overview.md) — the exact JSON
every binding (FFI, wasm, Python, Extism) emits.

```rust
use aozora::Document;
use aozora::wire::serialize_diagnostics;

fn main() {
    // U+E001 is a private-use sentinel the parser reserves; feeding one
    // in raises a diagnostic without aborting the parse.
    let doc = Document::new("abc\u{E001}def");
    let tree = doc.parse();

    let json = serialize_diagnostics(tree.diagnostics());
    println!("{json}");
}
```

> The `wire` module is behind the `wire` Cargo feature on `aozora`.

## Expected output

```json
{"schema_version":1,"data":[{"kind":"source_contains_pua","severity":"warning","source":"source","span":{"start":3,"end":6},"codepoint":""}]}
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

A JSON output mode for `check` (`--diagnostic-format json`, emitting
the same `serialize_diagnostics` envelope) is planned so scripts get
the structured stream without writing Rust. Until it lands, the
library path above is the supported way to obtain the JSON; the CLI's
current output is the human-readable form documented in the
[CLI reference](../ref/cli.md).

## See also

- Runnable example: **`just example diagnostics`**
  (`crates/aozora/examples/diagnostics.rs`).
- [Diagnostics catalogue](../notation/diagnostics.md) — every code,
  severity, and what triggers it.
- [Wire format](../wire/overview.md) — the envelope schema and version
  contract.
- [CLI reference → `aozora check`](../ref/cli.md) — flags and exit
  codes.
