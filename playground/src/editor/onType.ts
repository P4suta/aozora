import {
  Annotation,
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
  let cursorAfter: number | null = null;
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
    cursorAfter = fromB + spec.cursorOffset;
  });

  if (replacements.length === 0) return tr;
  return [
    tr,
    {
      changes: replacements,
      selection:
        cursorAfter !== null ? EditorSelection.cursor(cursorAfter) : undefined,
      annotations: ON_TYPE.of(true),
      sequential: true,
    },
  ];
});
