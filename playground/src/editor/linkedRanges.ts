import { Annotation, type ChangeSpec, EditorState } from '@codemirror/state';
import {
  byteToUtf16,
  type PairEntry,
  type ParserState,
  parserStateField,
} from './parserState';

const LINKED = Annotation.define<true>();
const MAX_LINKED_MARKER_BYTES = 4;

function isMinimalPair(p: PairEntry): boolean {
  return (
    p.open.end - p.open.start <= MAX_LINKED_MARKER_BYTES &&
    p.close.end - p.close.start <= MAX_LINKED_MARKER_BYTES
  );
}

/**
 * Delete the paired marker when a deletion covers one whole marker.
 * Each marker must fit within `MAX_LINKED_MARKER_BYTES`.
 * Insertions and replacements leave the paired marker unchanged.
 */
export const linkedRangesFilter = EditorState.transactionFilter.of((tr) => {
  if (!tr.docChanged) return tr;
  if (tr.annotation(LINKED)) return tr;

  const ps: ParserState = tr.startState.field(parserStateField);
  const entries = ps.pairs;
  if (entries.length === 0) return tr;

  const extras: ChangeSpec[] = [];

  tr.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
    if (toA <= fromA) return;
    if (inserted.length > 0) return;
    for (const pair of entries) {
      if (!isMinimalPair(pair)) continue;
      const openFrom = byteToUtf16(ps, pair.open.start);
      const openTo = byteToUtf16(ps, pair.open.end);
      const closeFrom = byteToUtf16(ps, pair.close.start);
      const closeTo = byteToUtf16(ps, pair.close.end);

      if (fromA <= openFrom && toA >= openTo && toA <= closeFrom) {
        extras.push({ from: closeFrom, to: closeTo, insert: '' });
      } else if (fromA <= closeFrom && toA >= closeTo && fromA >= openTo) {
        extras.push({ from: openFrom, to: openTo, insert: '' });
      }
    }
  });

  if (extras.length === 0) return tr;
  return [
    tr,
    {
      changes: extras,
      annotations: LINKED.of(true),
    },
  ];
});
