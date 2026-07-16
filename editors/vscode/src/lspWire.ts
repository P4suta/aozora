// Payload shapes for the `aozora/*` LSP custom requests.
//
// These are hand-written counterparts to the Rust structs the server
// serialises (`crates/aozora-lsp/src/backend.rs`), and there is no
// generator between them: the Rust side is `pub(crate)`, so producing
// these from it would mean re-exporting server internals, teaching
// `xtask` about `aozora-lsp`, and drift-gating the result — a lot of
// machinery for three payloads. They live here instead of beside their
// callers so that at least the TypeScript half has one definition:
// `RenderHtmlResult` was declared twice and the two copies had already
// started to diverge.
//
// The standard LSP types come from `vscode-languageclient`; only the
// custom methods need this file.

/** Response to `aozora/renderHtml`. */
export interface RenderHtmlResult {
  /** The rendered document, or a notice standing in for it — see `paused`. */
  readonly html: string;
  /**
   * `true` when the server declined to render and `html` is a notice
   * rather than the document (the oversize path). Surfaces that display
   * the notice are fine; anything that *persists* the render must refuse,
   * or it writes the notice and reports success.
   */
  readonly paused: boolean;
}

/** Response to `aozora/gaijiSpans`. */
export interface GaijiSpansResponse {
  readonly spans: ReadonlyArray<GaijiSpanWire>;
}

/** One `※［＃…］` reference the server resolved. */
export interface GaijiSpanWire {
  readonly range: { start: VsPositionLike; end: VsPositionLike };
  readonly resolved: string;
}

/** An LSP position: zero-based line, UTF-16 character offset. */
export interface VsPositionLike {
  readonly line: number;
  readonly character: number;
}
