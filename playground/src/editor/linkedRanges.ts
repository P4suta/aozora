import { Annotation, type ChangeSpec, EditorState } from '@codemirror/state';
import {
  byteToUtf16,
  type PairEntry,
  type ParserState,
  parserStateField,
} from './parserState';

/** Tag follow-up transactions so the filter does not recurse. */
const LINKED = Annotation.define<true>();

/**
 * Bracket pairs are considered "minimal" — and therefore safe to
 * mirror-edit — when each side is at most 4 UTF-8 bytes (one
 * full-width character or an ASCII pair). This excludes container
 * markers whose open/close spans cover entire `［＃...］` slugs.
 */
function isMinimalPair(p: PairEntry): boolean {
  return p.open.end - p.open.start <= 4 && p.close.end - p.close.start <= 4;
}

/**
 * Mirror-deletion of bracket pairs.
 *
 * When the user deletes the open marker (e.g. `《`) of a minimal
 * pair, also delete the matching close marker. Vice versa for the
 * close. Insertions and replacements are ignored — those have too
 * many failure modes (autocomplete, IME composition, structured
 * snippets), and the perceived value of mirroring inserts is low
 * compared to the cost of getting it wrong.
 *
 * The follow-up transaction carries the `LINKED` annotation so this
 * filter sees only the user's original edit, never its own mirror.
 */
export const linkedRangesFilter = EditorState.transactionFilter.of((tr) => {
  if (!tr.docChanged) return tr;
  if (tr.annotation(LINKED)) return tr;

  const ps: ParserState = tr.startState.field(parserStateField);
  const entries = ps.pairs;
  if (entries.length === 0) return tr;

  const extras: ChangeSpec[] = [];

  tr.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
    if (toA <= fromA) return; // pure insertion
    if (inserted.length > 0) return; // replacement
    for (const pair of entries) {
      if (!isMinimalPair(pair)) continue;
      const openFrom = byteToUtf16(ps, pair.open.start);
      const openTo = byteToUtf16(ps, pair.open.end);
      const closeFrom = byteToUtf16(ps, pair.close.start);
      const closeTo = byteToUtf16(ps, pair.close.end);

      // Deletion fully contains the open span → mirror by deleting close.
      if (fromA <= openFrom && toA >= openTo && toA <= closeFrom) {
        extras.push({ from: closeFrom, to: closeTo, insert: '' });
      }
      // Deletion fully contains the close span → mirror by deleting open.
      else if (fromA <= closeFrom && toA >= closeTo && fromA >= openTo) {
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
