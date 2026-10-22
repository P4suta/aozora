import {
  type Disposable,
  type ExtensionContext,
  Range,
  type TextDocument,
  type TextEditor,
  type TextEditorDecorationType,
  ThemeColor,
  window,
  workspace,
} from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

import { AsyncGeneration, anyPositionInRange, type PositionLike } from "./extensionLogic";
import { type GaijiSpansResponse, type GaijiSpanWire, parseGaijiSpansResponse } from "./lspWire";

const GAIJI_SPANS_REQUEST = "aozora/gaijiSpans" as const;
const CONFIG_NAMESPACE = "aozora.gaijiFold" as const;

class FoldState implements Disposable {
  private readonly hideDecoration: TextEditorDecorationType;
  private readonly glyphDecorations = new Map<string, TextEditorDecorationType>();
  private readonly refreshes = new WeakMap<TextEditor, AsyncGeneration>();
  private disposed = false;
  private readonly activeInlayDecorations = new Map<string, TextEditorDecorationType>();

  constructor() {
    this.hideDecoration = window.createTextEditorDecorationType({
      textDecoration: "none; font-size: 0px; letter-spacing: -1ch",
    });
  }

  async refresh(editor: TextEditor, client: LanguageClient): Promise<void> {
    if (editor.document.languageId !== "aozora") {
      this.clear(editor);
      return;
    }
    if (!isEnabled()) {
      this.clear(editor);
      return;
    }
    const document = editor.document;
    const version = document.version;
    const refresh = this.refreshes.get(editor) ?? new AsyncGeneration();
    this.refreshes.set(editor, refresh);
    const generation = refresh.begin();
    let response: GaijiSpansResponse;
    try {
      response = parseGaijiSpansResponse(
        await client.sendRequest<unknown>(GAIJI_SPANS_REQUEST, {
          uri: editor.document.uri.toString(),
        }),
      );
    } catch {
      if (
        !this.disposed &&
        refresh.isCurrent(generation) &&
        editor.document === document &&
        document.version === version
      ) {
        this.clear(editor);
      }
      return;
    }
    if (
      this.disposed ||
      !refresh.isCurrent(generation) ||
      editor.document !== document ||
      document.version !== version
    ) {
      return;
    }
    if (!spansFitDocument(document, response.spans)) {
      this.clear(editor);
      return;
    }
    const cursors = editor.selections.map((selection) => selection.active);
    const { hideRanges, glyphBuckets, activeInlayBuckets } = bucket(response.spans, cursors);
    editor.setDecorations(this.hideDecoration, hideRanges);
    this.applyBuckets(editor, this.glyphDecorations, glyphBuckets, createGlyphDecoration);
    this.applyBuckets(
      editor,
      this.activeInlayDecorations,
      activeInlayBuckets,
      createActiveInlayDecoration,
    );
  }

  clear(editor: TextEditor): void {
    this.refreshes.get(editor)?.invalidate();
    editor.setDecorations(this.hideDecoration, []);
    for (const deco of this.glyphDecorations.values()) {
      editor.setDecorations(deco, []);
    }
    for (const deco of this.activeInlayDecorations.values()) {
      editor.setDecorations(deco, []);
    }
  }

  dispose(): void {
    this.disposed = true;
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

function spansFitDocument(document: TextDocument, spans: ReadonlyArray<GaijiSpanWire>): boolean {
  return spans.every((span) => {
    const range = new Range(
      span.range.start.line,
      span.range.start.character,
      span.range.end.line,
      span.range.end.character,
    );
    return range.isEqual(document.validateRange(range));
  });
}

function bucket(
  spans: ReadonlyArray<GaijiSpanWire>,
  cursors: ReadonlyArray<PositionLike>,
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
    if (anyPositionInRange(cursors, range)) {
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
      for (const editor of window.visibleTextEditors) {
        if (editor.document === event.document) {
          refresh(editor);
        }
      }
    }),
    window.onDidChangeVisibleTextEditors((editors) => {
      for (const editor of editors) {
        refresh(editor);
      }
    }),
    window.onDidChangeTextEditorSelection((event) => refresh(event.textEditor)),
    workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration(CONFIG_NAMESPACE)) {
        for (const editor of window.visibleTextEditors) {
          refresh(editor);
        }
      }
    }),
  );

  for (const editor of window.visibleTextEditors) {
    refresh(editor);
  }
}
