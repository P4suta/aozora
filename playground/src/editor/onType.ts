import {
  Annotation,
  ChangeSet,
  type ChangeSpec,
  EditorSelection,
  EditorState,
} from '@codemirror/state';

interface ReplacementSpec {
  insert: string;
  cursorOffset: number;
}

const HALF_TO_FULL: Record<string, ReplacementSpec> = {
  '[': { insert: '［＃］', cursorOffset: 2 },
  '<': { insert: '《》', cursorOffset: 1 },
  '{': { insert: '〔〕', cursorOffset: 1 },
  ']': { insert: '］', cursorOffset: 1 },
  '>': { insert: '》', cursorOffset: 1 },
  '}': { insert: '〕', cursorOffset: 1 },
  '|': { insert: '｜', cursorOffset: 1 },
  '*': { insert: '※', cursorOffset: 1 },
  '#': { insert: '＃', cursorOffset: 1 },
};

/** Mark the follow-up rewrite so the filter does not re-enter. */
const ON_TYPE = Annotation.define<true>();

/**
 * Single-char half-width inserts are rewritten to their full-width
 * counterparts, optionally with a paired closer. IME composition
 * events are skipped: their changes arrive in larger chunks and
 * tend to land outside this filter's scope anyway.
 */
export const halfToFullWidthFilter = EditorState.transactionFilter.of((tr) => {
  if (!tr.docChanged) return tr;
  if (tr.annotation(ON_TYPE)) return tr;
  // IME composition is multi-step; do not interfere.
  if (tr.isUserEvent('input.compose')) return tr;

  const replacements: ChangeSpec[] = [];
  const cursorOffsets = new Map<number, number>();
  tr.changes.iterChanges((fromA, toA, fromB, _toB, inserted) => {
    if (toA !== fromA) return; // pure insertion only
    if (inserted.length !== 1) return;
    const text = inserted.sliceString(0);
    if (text.length !== 1) return;
    const spec = HALF_TO_FULL[text];
    if (!spec) return;
    replacements.push({
      from: fromB,
      to: fromB + text.length,
      insert: spec.insert,
    });
    cursorOffsets.set(fromB + text.length, spec.cursorOffset);
  });

  if (replacements.length === 0) return tr;
  const rewritten = ChangeSet.of(replacements, tr.newDoc.length);
  const mappedSelection = tr.newSelection.map(rewritten);
  const ranges = tr.newSelection.ranges.map((range, index) => {
    const cursorOffset = range.empty
      ? cursorOffsets.get(range.head)
      : undefined;
    if (cursorOffset === undefined) return mappedSelection.ranges[index]!;
    const start = rewritten.mapPos(range.head - 1, -1);
    return EditorSelection.cursor(start + cursorOffset);
  });
  return [
    tr,
    {
      changes: rewritten,
      selection: EditorSelection.create(ranges, tr.newSelection.mainIndex),
      annotations: ON_TYPE.of(true),
      sequential: true,
    },
  ];
});
