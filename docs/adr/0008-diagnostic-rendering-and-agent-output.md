# 0008. Diagnostic rendering & the agent-facing output contract

- Status: accepted
- Date: 2026-06-15
- Deciders: @P4suta
- Tags: diagnostics, cli, agents, miette, wire

## Context

The parser is non-fatal by design: it always produces a tree and reports
what it noticed through structured `aozora-spec::Diagnostic` values, each
carrying a stable string `code`, a `severity`, a source `axis`
(`Source` vs `Internal`), and a `span`. That structure was rich, but the
front door wasted it.

Two concrete problems:

1. **The CLI threw the structure away.** `aozora check` printed each
   diagnostic with a bare `writeln!("{diag}")` — `Display` only, one line,
   no source line, no caret, no help. The `Diagnostic` derive already
   modelled `miette`'s graphical machinery (labels, help, source span),
   but nothing ever attached the source, so the graphical render was dead
   code.

2. **`miette`'s `fancy` feature had leaked into the library tree.** The
   workspace pinned `miette { features = ["fancy"] }`, so `aozora-spec` —
   and therefore every downstream *library* consumer (afm, the WASM
   package, the Python wheel) — transitively compiled the graphical
   renderer's dependency tail: `backtrace`, `supports-color`,
   `terminal_size`, `textwrap`. A library that never renders a terminal
   report was paying for one.

Separately, agents and editors increasingly drive the binary
(stdin → stdout). They need a *deterministic, machine-described* view of
diagnostics and a *meaningful exit code*, not prose scraped from stderr.

## Decision

1. **Three rendering views, auto-selected.** `aozora check` gains
   `--diagnostic-format {human,json,short}`, defaulting to `Auto`:
   - **`human`** (default when stderr is a TTY) — build a
     `miette::Report` with the source attached and render the graphical
     snippet + caret + label + help (`fancy`).
   - **`json`** (default when stderr is piped) — emit
     `aozora::wire::serialize_diagnostics`, byte-identical to what the
     FFI / WASM / PyO3 / Extism front doors produce. This is the
     machine / agent path; piping makes it the default with no flag.
   - **`short`** — one grep-able line per diagnostic,
     `path:offset: severity[code]: message`, for editors that draw their
     own snippets.

2. **Spans are in sanitized coordinates, and the renderer attaches the
   sanitized text.** Diagnostic spans are byte ranges in the **Phase 0
   sanitized** source (BOM stripped, CRLF→LF folded, 〔…〕 accents
   decomposed), *not* the raw input. The `human` renderer therefore
   re-derives the sanitized bytes (`aozora::pipeline::lexer::sanitize`)
   and attaches *those* as the report's source, so the caret lands on the
   right column even for CRLF Aozora Bunko files — attaching the raw
   bytes would slide every caret right by the count of preceding line
   breaks. (The earlier doc comments claiming the span was
   "original / pre-normalization" were wrong and were corrected;
   a CRLF input empirically reports the sanitized offset.)

3. **`miette` feature hygiene.** The workspace pins **bare** `miette`
   (the `Diagnostic` derive + `SourceSpan` only); only `aozora-cli` opts
   into `features = ["fancy"]`. Library consumers no longer compile the
   graphical-renderer dependency tail.

4. **A documented exit-code contract:**
   - `0` — diagnostics tolerated (printed, but not fatal).
   - `1` — `--strict` and ≥ 1 diagnostic fired.
   - `2` — CLI usage error (bad flags / arguments).
   - `3` — an `Internal`-axis diagnostic fired: a library bug, held
     distinct from bad input so CI can tell "your file is malformed" from
     "aozora is malformed".

   The contract is stated in `aozora check --help`, the handbook CLI
   reference, and (FUTURE) the repo's `AGENTS.md`.

5. **Codes link to docs.** Each `Diagnostic` variant carries a `miette`
   `url(...)` pointing at its anchor in the handbook diagnostics
   catalogue, so a rendered report links straight to the explanation.

6. **The parser describes the symptom, not the cure.** Diagnostics state
   *what* was noticed (unclosed bracket, unresolved gaiji); they do not
   propose edits. Actionable fix-its / suggestions stay an opt-in higher
   layer (a formatter / LSP), consistent with the zero-parser-hooks thesis
   (ADR-0001) and the error-recovery model — the core stays a pure
   reporter.

7. **No MCP / `serve` mode — REJECTED for now.** stdin → stdout plus
   `--diagnostic-format json` plus the stable exit codes already give
   agents and editors a deterministic, idiomatic interface, and it
   preserves the "single binary, no runtime process dependencies"
   guarantee. A long-lived sub-process protocol is reconsidered only if a
   real integration needs sub-process-call latency or incremental parse
   state (mirrors the playground roadmap's rejection of LSP-in-WASM).

## Consequences

- **The graphical render is alive and correct.** A human running
  `aozora check file.txt` on a terminal gets the source line, a caret on
  the right column (even on CRLF), the label, the help, and a link to the
  catalogue — none of which the old `Display`-only path produced.
- **Libraries shed a dependency tail.** Dropping `fancy` from the shared
  pin removes `backtrace` / `supports-color` / `terminal_size` /
  `textwrap` from afm, the WASM bundle, and the Python wheel. Only the CLI
  binary carries the graphical renderer.
- **Agents get a no-flag stable stream.** Because `Auto` resolves to
  `json` whenever stderr is not a TTY, a piped or captured invocation
  emits the versioned `wire` envelope automatically; combined with the
  exit codes, a CI step or agent can branch on outcome without parsing
  prose.
- **The sanitized-coordinate invariant is now load-bearing.** Anything
  that consumes a span — the renderer, an editor overlay, a future
  fix-it — must map through Phase 0, not the raw bytes. This is a contract
  the catalogue and the renderer's source comment both record.
- **`json` is bound to `SCHEMA_VERSION`.** The agent view rides the same
  `aozora::wire` envelope as every binding (ADR-0006), so a schema bump
  versions the CLI's machine output in lockstep with the SDKs.
- **The catalogue is a roadmap, not just a reference.** The handbook
  specifies ~12 authoring-error diagnostics (empty ruby, nested ruby,
  ambiguous bouten target, unresolved gaiji, kaeriten-outside-kanbun, …).
  The parser emits **3 `Source` + 4 `Internal`** codes today; the rest are
  marked **Planned**. This ADR fixes the rendering / contract foundation
  those future detections plug into — each lands as a `Diagnostic`
  variant + detection in the owning pipeline phase + a spec fixture,
  needing no further rendering work.

## Alternatives considered

**Keep `Display`-only printing.** One line per diagnostic, no source
context. Cheap, but it discards the span and severity that make a report
useful to a human and unusable to a machine, and it left the
already-compiled graphical machinery dead. Rejected.

**Keep `fancy` in the shared pin (library renders too).** Lets any
consumer call `{:?}` for a graphical report. Rejected: it taxes every
downstream *library* with a terminal-rendering dependency tail they never
exercise; rendering is a CLI concern, so the `fancy` opt-in lives only in
`aozora-cli`.

**Attach the raw (pre-sanitize) source to the report.** Simpler — no
re-derivation step. Rejected: spans are in sanitized coordinates, so on a
CRLF file (the Aozora Bunko norm) every caret would drift right by the
number of preceding line breaks. The renderer re-runs `sanitize` so the
attached bytes are exactly the bytes the lexer spanned into.

**An MCP server / persistent `serve` mode.** A long-lived process
exposing parse/check over a protocol. Rejected for now: it adds a runtime
process dependency (breaking the single-binary guarantee) and a stateful
surface, while the stateless stdin→stdout + JSON + exit-code path is
already deterministic and idiomatic for agents and editors. Revisit only
on a concrete latency / incremental-state need.

**Emit fix-it suggestions from the parser.** Richer authoring help, but
it couples the core to edit heuristics and a presentation model, against
ADR-0001's separation. Rejected: the parser reports the symptom; the cure
belongs to an opt-in higher layer (formatter / LSP).

## References

- CLI renderer: `crates/aozora-cli/src/diagnostics_render.rs`
  (the three views; `Auto` → TTY check; sanitized-source attach).
- Wire authority (machine view): `crates/aozora/src/wire.rs`
  (`serialize_diagnostics`, `SCHEMA_VERSION`).
- Sanitize phase (span coordinate origin):
  `crates/aozora-pipeline/src/lexer/sanitize.rs`.
- miette feature pins: workspace `Cargo.toml` (bare `miette`) vs
  `crates/aozora-cli/Cargo.toml` (`features = ["fancy"]`); library example
  `crates/aozora-spec/Cargo.toml` (bare).
- Handbook: `crates/aozora-book/src/notation/diagnostics.md` (catalogue +
  Planned table), `crates/aozora-book/src/ref/cli.md` (exit-code
  contract), `crates/aozora-book/src/arch/error-recovery.md`,
  `crates/aozora-book/src/recipes/diagnostics-json.md`.
- Related: ADR-0001 (zero parser hooks — symptom-not-cure, compose
  downstream), ADR-0006 (polyglot bindings — the shared `wire` envelope
  the `json` view emits), ADR-0003 (accent decomposition — a Phase 0
  transform that moves span offsets).
