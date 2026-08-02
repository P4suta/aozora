// Payload shapes for the `aozora/*` LSP custom requests.
//
// These are hand-written counterparts to the Rust structs the server
// serialises (`crates/aozora-cli/src/lsp/backend.rs`), and there is no
// generator between them: the Rust side is `pub(crate)`, so producing
// these from it would mean re-exporting server internals, teaching
// `xtask` about the in-process language server, and drift-gating the result — a lot of
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

interface JsonShape {
  readonly character?: unknown;
  readonly end?: unknown;
  readonly html?: unknown;
  readonly line?: unknown;
  readonly paused?: unknown;
  readonly range?: unknown;
  readonly resolved?: unknown;
  readonly spans?: unknown;
  readonly start?: unknown;
}

export function parseRenderHtmlResult(value: unknown): RenderHtmlResult {
  const result = record(value, "aozora/renderHtml");
  if (typeof result.html !== "string" || typeof result.paused !== "boolean") {
    throw new Error("invalid aozora/renderHtml response");
  }
  return { html: result.html, paused: result.paused };
}

export function parseGaijiSpansResponse(value: unknown): GaijiSpansResponse {
  const result = record(value, "aozora/gaijiSpans");
  if (!Array.isArray(result.spans)) {
    throw new Error("invalid aozora/gaijiSpans response");
  }
  return {
    spans: result.spans.map((value) => {
      const span = record(value, "aozora/gaijiSpans");
      const range = record(span.range, "aozora/gaijiSpans");
      const start = position(range.start);
      const end = position(range.end);
      if (
        typeof span.resolved !== "string" ||
        span.resolved.length === 0 ||
        comparePositions(start, end) > 0
      ) {
        throw new Error("invalid aozora/gaijiSpans response");
      }
      return { range: { start, end }, resolved: span.resolved };
    }),
  };
}

function record(value: unknown, method: string): JsonShape {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`invalid ${method} response`);
  }
  return value as JsonShape;
}

function position(value: unknown): VsPositionLike {
  const candidate = record(value, "aozora/gaijiSpans");
  if (!(isCoordinate(candidate.line) && isCoordinate(candidate.character))) {
    throw new Error("invalid aozora/gaijiSpans response");
  }
  return { line: candidate.line, character: candidate.character };
}

function isCoordinate(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function comparePositions(left: VsPositionLike, right: VsPositionLike): number {
  return left.line - right.line || left.character - right.character;
}
