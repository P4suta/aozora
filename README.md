# aozora

Pure-functional Rust parser for **青空文庫記法** (Aozora Bunko notation):
ruby (`｜青梅《おうめ》`), bouten (`［＃「X」に傍点］`), 縦中横, 外字
references (`※［＃…、第3水準1-85-54］`), kunten / kaeriten,
indent / align-end containers (`［＃ここから2字下げ］…
［＃ここで字下げ終わり］`), page and section breaks.

The parser is **comrak-free, CommonMark-free, and Markdown-free** —
this repo deals only with the 青空文庫 notation itself. The CommonMark+GFM
Markdown dialect that layers on top of this parser lives in the sibling
[`afm`](https://github.com/P4suta/afm) repo (the *Aozora Flavored
Markdown* hobby project that motivated the split). The authoring
environment (formatter, LSP, VS Code extension) lives in
[`aozora-tools`](https://github.com/P4suta/aozora-tools).

## Crate layout

```
crates/aozora-syntax       AST types (AozoraNode + variants, accent table)
crates/aozora-lexer        Pure-functional 7-phase lexer; produces a
                           PUA-sentinel-normalized text + placeholder
                           registry (no recogniser hooks; ADR-0001)
crates/aozora-parser       parse / serialize / html::render_to_string —
                           single-pass match_indices walker over the
                           registry (no tree allocation in v0.1.0)
crates/aozora-encoding     Shift_JIS decoding + 外字 lookup (compile-time
                           PHF table for UCS resolution)
crates/aozora-corpus       Corpus source abstraction for sweep tests
                           (dev-only, set AOZORA_CORPUS_ROOT)
crates/aozora-cli          `aozora` binary: check / fmt / render
crates/aozora-test-utils   Shared proptest strategies (dev-only)
```

## Quickstart

Everything runs inside Docker — host toolchain is never invoked. Bring up
the dev image once, then drive every operation through `just`:

```sh
just                    # list targets
just build              # cargo build --workspace
just test               # cargo nextest run --workspace
just spec-aozora        # hand-written annotation fixtures
just spec-golden-56656  # 罪と罰 (card 56656) Tier-A acceptance gate
just lint               # fmt + clippy + typos + strict-code purity check
just ci                 # full CI replica (gates on all of the above)
```

Use `just run` to invoke the CLI:

```sh
just run check FILE.txt           # lex + report diagnostics
just run fmt --check FILE.txt     # round-trip parse ∘ serialize check
just run render FILE.txt          # render to HTML on stdout
just run check -E sjis FILE.txt   # Shift_JIS source from Aozora Bunko
```

## Public API contract

The library surface mirrors what the sibling `aozora-tools` and `afm`
repos depend on:

```rust
use aozora_parser::{parse, serialize, html, ParseResult, ParseArtifacts};
use aozora_syntax::{AozoraNode, ContainerKind, Ruby, Bouten, …};
use aozora_encoding::{decode_sjis, gaiji};

let result = parse("｜青梅《おうめ》");
assert!(result.diagnostics.is_empty());
let canonical = serialize(&result);
let html = html::render_to_string("｜青梅《おうめ》");
```

`parse(input) -> ParseResult` is the single front door. The result
carries lexer diagnostics and the raw `(normalized, registry)`
artifacts; `serialize` and `html::render_to_string` invert / render
straight off those artifacts in `O(n)`.

ADR-0001 (zero parser hooks) is the architectural thesis: every Aozora
recogniser is a pure function in `aozora_lexer`'s 7-phase pipeline.
The parser only consumes the lexer's output.

## Three-layer ecosystem

```
┌─────────────────────────────────────────┐
│ aozora-tools  (authoring environment)    │
│   crates/aozora-fmt    formatter CLI     │
│   crates/aozora-lsp    LSP server        │
│   editors/vscode/      TS client         │
└──────────────┬──────────────────────────┘
               │ depends on aozora directly
               ▼
┌─────────────────────────────────────────┐
│ afm  (CommonMark + GFM + aozora dialect) │
│   crates/afm-markdown  Markdown layer    │
│   crates/afm-cli       afm render/check  │
│   upstream/comrak/     vendored fork     │
└──────────────┬──────────────────────────┘
               │ depends on aozora via git tag
               ▼
┌─────────────────────────────────────────┐
│ aozora  (this repo — pure 青空文庫記法)   │
└─────────────────────────────────────────┘
```

See [`docs/adr/0006-authoring-tools-live-in-sibling-repositories.md`](docs/adr/0006-authoring-tools-live-in-sibling-repositories.md)
for the rationale.

## License

Apache-2.0 OR MIT (see [`LICENSE-APACHE`](LICENSE-APACHE) /
[`LICENSE-MIT`](LICENSE-MIT)).
