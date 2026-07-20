import { RangeSetBuilder } from '@codemirror/state';
import {
  Decoration,
  EditorView,
  ViewPlugin,
  type DecorationSet,
  type ViewUpdate,
} from '@codemirror/view';
import {
  byteToUtf16,
  parserStateField,
  utf16ToByte,
  type ParserState,
} from './parserState';
import { lowerBoundByStart } from './utils';

/**
 * Map every `kind` returned by `Document.nodes()` to a CSS class
 * defined in `theme.ts`. Kinds not in this table are skipped.
 *
 * Mapping note: the wire format uses camelCase ("aozoraHeading"); we
 * fold the prefix here so the CSS class names stay short and readable.
 */
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

  // Find the slice of entries that could overlap the viewport.
  // Widen by 32 entries on the leading edge because entries are
  // sorted by start, and an earlier-starting entry may still cover
  // bytes inside our viewport.
  const startIdx = Math.max(0, lowerBoundByStart(entries, vpFromByte) - 32);

  // Decorations must be added in increasing `from` order. Since
  // entries are sorted by span.start, and span.start in bytes maps
  // monotonically to UTF-16 positions, the resulting `from` values
  // are also non-decreasing.
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
