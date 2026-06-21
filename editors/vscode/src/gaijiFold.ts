// Inline-fold decorations for `※［＃description、mencode］` spans.
//
// VS Code shows the verbose source bytes by default — for prose
// reading that is all noise. The plugin replaces each gaiji span
// with the resolved character so reading flows; the source
// re-appears the moment the cursor enters the span.
//
// ## Two display modes per span
//
// - **Folded** (cursor outside the span): source is hidden and
//   the resolved glyph is shown in its place. Reading view.
// - **Active** (cursor inside the span): source is fully visible
//   for editing, and a subtle `→ X` annotation appears just after
//   the closing ］ to confirm what the source resolves to.
//
// Both modes are driven by the same `aozora/gaijiSpans` LSP
// request payload — the cursor position decides per-span which
// mode applies. There's no LSP-side inlay because VS Code's
// inlay layer cannot be suppressed by a decoration; mixing the
// two would surface the `→ X` glyph twice on every folded span.
//
// ## Architecture
//
// 1. Ask the LSP for every resolvable span (lock-free read against
//    the pre-extracted gaiji cache; microseconds per call).
// 2. Bucket the spans by the resolved glyph string and share one
//    [`TextEditorDecorationType`] per glyph (`O(distinct glyphs)`
//    rather than `O(spans)` decoration handles).
// 3. Apply two layers per *folded* span:
//      - the *hide* layer collapses the source via CSS
//        `font-size: 0` + `letter-spacing: -1ch` (no API truly
//        removes the chars from the editor model);
//      - the *glyph* layer's `before` decoration shows the
//        resolved char with a subtle dotted underline.
// 4. For the *active* span, neither hide nor glyph decoration
//    apply (so source is visible); instead a third decoration
//    type emits an `after` annotation with `→ X`.

import {
  type Disposable,
  type ExtensionContext,
  Range,
  type TextEditor,
  type TextEditorDecorationType,
  ThemeColor,
  type Position as VsPosition,
  window,
  workspace,
} from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

/** Server-side payload for the `aozora/gaijiSpans` LSP request.
 *  Mirrors `aozora_lsp::backend::GaijiSpansResult`. */
interface GaijiSpansResponse {
  readonly spans: ReadonlyArray<GaijiSpanWire>;
}

interface GaijiSpanWire {
  readonly range: { start: VsPositionLike; end: VsPositionLike };
  readonly resolved: string;
  readonly description: string;
  readonly mencode: string | null;
}

interface VsPositionLike {
  readonly line: number;
  readonly character: number;
}

const GAIJI_SPANS_REQUEST = "aozora/gaijiSpans" as const;
const CONFIG_NAMESPACE = "aozora.gaijiFold" as const;

/** Per-extension instance state. Holds the decoration types so we
 *  can reuse them across refreshes (creating a new type per call
 *  would balloon the editor's decoration handle table). */
class FoldState implements Disposable {
  private readonly hideDecoration: TextEditorDecorationType;
  private readonly glyphDecorations = new Map<string, TextEditorDecorationType>();
  /**
   * Per-resolved-glyph "active inlay" decorations — emit an
   * `after` annotation reading "→ X" when the cursor sits inside
   * a span. Bucketed per glyph for the same reason as
   * `glyphDecorations`; rebuilt lazily on first sight of each
   * resolved string.
   */
  private readonly activeInlayDecorations = new Map<string, TextEditorDecorationType>();

  constructor() {
    this.hideDecoration = window.createTextEditorDecorationType({
      // No API removes characters from the editor; collapse them
      // visually to zero width via CSS instead.
      textDecoration: "none; font-size: 0px; letter-spacing: -1ch",
    });
  }

  /** Fetch and apply spans for `editor`. Idempotent — safe to call
   *  on every selection / didChange / config event. */
  async refresh(editor: TextEditor, client: LanguageClient): Promise<void> {
    if (editor.document.languageId !== "aozora") {
      return;
    }
    if (!isEnabled()) {
      this.clear(editor);
      return;
    }
    let response: GaijiSpansResponse;
    try {
      response = await client.sendRequest<GaijiSpansResponse>(GAIJI_SPANS_REQUEST, {
        uri: editor.document.uri.toString(),
      });
    } catch {
      // Server is briefly unavailable (start-up, mid-restart) — skip
      // this refresh; the next event will retry.
      return;
    }
    const cursor = editor.selection.active;
    const { hideRanges, glyphBuckets, activeInlayBuckets } = bucket(response.spans, cursor);
    editor.setDecorations(this.hideDecoration, hideRanges);
    this.applyBuckets(editor, this.glyphDecorations, glyphBuckets, createGlyphDecoration);
    this.applyBuckets(
      editor,
      this.activeInlayDecorations,
      activeInlayBuckets,
      createActiveInlayDecoration,
    );
  }

  /** Clear every decoration this state owns from `editor`. */
  clear(editor: TextEditor): void {
    editor.setDecorations(this.hideDecoration, []);
    for (const deco of this.glyphDecorations.values()) {
      editor.setDecorations(deco, []);
    }
    for (const deco of this.activeInlayDecorations.values()) {
      editor.setDecorations(deco, []);
    }
  }

  dispose(): void {
    this.hideDecoration.dispose();
    for (const deco of this.glyphDecorations.values()) {
      deco.dispose();
    }
    for (const deco of this.activeInlayDecorations.values()) {
      deco.dispose();
    }
    this.glyphDecorations.clear();
    this.activeInlayDecorations.clear();
  }

  /**
   * Apply per-glyph buckets to `editor`. `store` is the long-lived
   * decoration cache; `factory` is invoked lazily for glyphs not
   * yet seen. Glyphs whose buckets are empty this cycle have their
   * ranges cleared but the decoration *type* is kept (the same
   * glyph likely reappears soon, and creating types is not free).
   */
  private applyBuckets(
    editor: TextEditor,
    store: Map<string, TextEditorDecorationType>,
    buckets: ReadonlyMap<string, ReadonlyArray<Range>>,
    factory: (glyph: string) => TextEditorDecorationType,
  ): void {
    const seen = new Set<string>();
    for (const [glyph, ranges] of buckets) {
      seen.add(glyph);
      let deco = store.get(glyph);
      if (!deco) {
        deco = factory(glyph);
        store.set(glyph, deco);
      }
      editor.setDecorations(deco, ranges);
    }
    for (const [glyph, deco] of store) {
      if (!seen.has(glyph)) {
        editor.setDecorations(deco, []);
      }
    }
  }
}

function bucket(
  spans: ReadonlyArray<GaijiSpanWire>,
  cursor: VsPosition,
): {
  hideRanges: Range[];
  glyphBuckets: Map<string, Range[]>;
  activeInlayBuckets: Map<string, Range[]>;
} {
  const hideRanges: Range[] = [];
  const glyphBuckets = new Map<string, Range[]>();
  const activeInlayBuckets = new Map<string, Range[]>();
  for (const span of spans) {
    const range = new Range(
      span.range.start.line,
      span.range.start.character,
      span.range.end.line,
      span.range.end.character,
    );
    if (range.contains(cursor)) {
      // Cursor sits inside — span stays unfolded, but we still
      // want a confirmation of what it resolves to. Render an
      // `after` decoration on a zero-width range at the closing
      // bracket so the editor draws "→ X" right past the source.
      const tail = new Range(range.end, range.end);
      const existing = activeInlayBuckets.get(span.resolved);
      if (existing) {
        existing.push(tail);
      } else {
        activeInlayBuckets.set(span.resolved, [tail]);
      }
      continue;
    }
    hideRanges.push(range);
    const existing = glyphBuckets.get(span.resolved);
    if (existing) {
      existing.push(range);
    } else {
      glyphBuckets.set(span.resolved, [range]);
    }
  }
  return { hideRanges, glyphBuckets, activeInlayBuckets };
}

function createGlyphDecoration(resolved: string): TextEditorDecorationType {
  return window.createTextEditorDecorationType({
    before: {
      contentText: resolved,
      color: new ThemeColor("editor.foreground"),
      textDecoration: "underline dotted rgba(128,128,128,0.6)",
      margin: "0",
    },
  });
}

function createActiveInlayDecoration(resolved: string): TextEditorDecorationType {
  return window.createTextEditorDecorationType({
    after: {
      contentText: ` → ${resolved}`,
      color: new ThemeColor("editorCodeLens.foreground"),
      fontStyle: "italic",
      margin: "0 0 0 0.2em",
    },
  });
}

function isEnabled(): boolean {
  return workspace.getConfiguration(CONFIG_NAMESPACE).get<boolean>("enabled", true);
}

export function registerGaijiFold(context: ExtensionContext, client: LanguageClient): void {
  const state = new FoldState();
  context.subscriptions.push(state);

  const refresh = (editor: TextEditor | undefined) => {
    if (!editor) {
      return;
    }
    void state.refresh(editor, client);
  };

  context.subscriptions.push(
    window.onDidChangeActiveTextEditor((editor) => refresh(editor)),
    workspace.onDidChangeTextDocument((event) => {
      const editor = window.activeTextEditor;
      if (editor && editor.document === event.document) {
        refresh(editor);
      }
    }),
    window.onDidChangeTextEditorSelection((event) => refresh(event.textEditor)),
    workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration(CONFIG_NAMESPACE)) {
        refresh(window.activeTextEditor);
      }
    }),
  );

  // Initial pass over editors that were already open when the
  // extension activated.
  for (const editor of window.visibleTextEditors) {
    refresh(editor);
  }
}
