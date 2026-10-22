import { RangeSetBuilder } from '@codemirror/state';
import {
  Decoration,
  type DecorationSet,
  type EditorView,
  ViewPlugin,
  type ViewUpdate,
} from '@codemirror/view';
import {
  byteToUtf16,
  type ParserState,
  parserStateField,
  utf16ToByte,
} from './parserState';
import { lowerBoundByStart } from './utils';

const VIEWPORT_LOOKBEHIND_ENTRIES = 32;

const KIND_TO_CLASS: Record<string, string> = {
  ruby: 'cm-aozora-ruby',
  angleQuote: 'cm-aozora-angle-quote',
  bouten: 'cm-aozora-bouten',
  gaiji: 'cm-aozora-gaiji',
  combineUpright: 'cm-aozora-combine-upright',
  illustration: 'cm-aozora-illustration',
  warichu: 'cm-aozora-warichu',
  kaeriten: 'cm-aozora-kaeriten',
  directive: 'cm-aozora-directive',
  heading: 'cm-aozora-aozora-heading',
  headingHint: 'cm-aozora-heading-hint',
  sectionBreak: 'cm-aozora-section-break',
  pageBreak: 'cm-aozora-page-break',
  containerOpen: 'cm-aozora-container-marker',
  containerClose: 'cm-aozora-container-marker',
};

function buildDecorations(view: EditorView): DecorationSet {
  const ps: ParserState = view.state.field(parserStateField);
  if (!ps.source) return Decoration.none;

  const entries = ps.nodes;
  if (entries.length === 0) return Decoration.none;

  const viewport = view.viewport;
  const vpFromByte = utf16ToByte(ps, viewport.from);
  const vpToByte = utf16ToByte(ps, viewport.to);

  const startIdx = Math.max(
    0,
    lowerBoundByStart(entries, vpFromByte) - VIEWPORT_LOOKBEHIND_ENTRIES,
  );

  const builder = new RangeSetBuilder<Decoration>();
  for (let i = startIdx; i < entries.length; i++) {
    const entry = entries[i]!;
    if (entry.span.start > vpToByte) break;
    if (entry.span.end < vpFromByte) continue;
    const cls = KIND_TO_CLASS[entry.kind];
    if (!cls) continue;
    const from = byteToUtf16(ps, entry.span.start);
    const to = byteToUtf16(ps, entry.span.end);
    if (from >= to) continue;
    builder.add(from, to, Decoration.mark({ class: cls }));
  }
  return builder.finish();
}

export const aozoraDecorations = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = buildDecorations(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged) {
        this.decorations = buildDecorations(update.view);
      }
    }
  },
  {
    decorations: (v) => v.decorations,
  },
);
