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

const ON_TYPE = Annotation.define<true>();

/** Leaves IME composition events unchanged. */
export const halfToFullWidthFilter = EditorState.transactionFilter.of((tr) => {
  if (!tr.docChanged) return tr;
  if (tr.annotation(ON_TYPE)) return tr;
  if (tr.isUserEvent('input.compose')) return tr;

  const replacements: ChangeSpec[] = [];
  const cursorOffsets = new Map<number, number>();
  tr.changes.iterChanges((fromA, toA, fromB, _toB, inserted) => {
    if (toA !== fromA) return;
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
