# aozora-diagnostics

Renderer-agnostic diagnostic catalogue for
[aozora-flavored-markdown](https://github.com/P4suta/aozora-tools).

The aozora lexer emits diagnostics (unclosed brackets, stray closes, private-use
codepoints, …). This crate turns each one into a `Described` record — a stable
`code`, an error/warning `Severity`, the verbose Japanese message, the source
`Span`, and a quick-fix payload — **without depending on any LSP or terminal
types**. Both [`aozora-lsp`](../aozora-lsp) (→ LSP `Diagnostic`) and the `aozora`
CLI (→ terminal output) consume this single source of truth, so the catalogue
never forks.

It also ships `CATALOGUE` / `lookup`, the long-form `explain` text behind the
`aozora explain <code>` command.

```rust
use aozora_diagnostics::{describe_source, Severity};

let diags = describe_source("本文［＃改ページ");
assert_eq!(diags[0].code, "aozora::unclosed-bracket");
assert_eq!(diags[0].severity, Severity::Error);
```

Part of the [aozora-tools](https://github.com/P4suta/aozora-tools) workspace.
Licensed under Apache-2.0 OR MIT.
