import {
  Decoration,
  EditorView,
  ViewPlugin,
  WidgetType,
  type DecorationSet,
  type ViewUpdate,
} from '@codemirror/view';
import { byteToUtf16, parserStateField, type ParserState } from './parserState';

class GaijiInlayWidget extends WidgetType {
  constructor(
    readonly resolved: string,
    readonly codepoint: number | null,
    readonly description: string,
  ) {
    super();
  }

  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-aozora-inlay';
    span.textContent = `→${this.resolved}`;
    const cpHex =
      this.codepoint !== null
        ? `U+${this.codepoint.toString(16).toUpperCase().padStart(4, '0')}`
        : '';
    span.title = cpHex ? `${this.description} (${cpHex})` : this.description;
    return span;
  }

  eq(other: GaijiInlayWidget): boolean {
    return (
      other.resolved === this.resolved &&
      other.codepoint === this.codepoint &&
      other.description === this.description
    );
  }

  ignoreEvent(): boolean {
    return false;
  }
}

function buildInlays(view: EditorView): DecorationSet {
  const ps: ParserState = view.state.field(parserStateField);
  const entries = ps.gaijiResolutions;
  if (entries.length === 0) return Decoration.none;
  const docLen = view.state.doc.length;
  // Use a sparse array we can sort, since gaiji entries from the
  // WASM scan come in source order but Decoration.set wants the
  // ranges already ordered by `from`.
  const items: { pos: number; widget: GaijiInlayWidget }[] = [];
  for (const r of entries) {
    if (!r.resolved) continue;
    const pos = byteToUtf16(ps, r.span.end);
    if (pos < 0 || pos > docLen) continue;
    items.push({
      pos,
      widget: new GaijiInlayWidget(r.resolved, r.codepoint, r.description),
    });
  }
  items.sort((a, b) => a.pos - b.pos);
  const decos = items.map((it) =>
    Decoration.widget({ widget: it.widget, side: 1 }).range(it.pos),
  );
  return Decoration.set(decos, true);
}

export const aozoraInlayHints = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = buildInlays(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged) {
        this.decorations = buildInlays(update.view);
      }
    }
  },
  {
    decorations: (v) => v.decorations,
  },
);
