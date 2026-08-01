import { EditorSelection, EditorState } from '@codemirror/state';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { Document } from '../wasm-loader';
import { linkedRangesFilter } from './linkedRanges';
import { halfToFullWidthFilter } from './onType';
import { parserStateField } from './parserState';

afterEach(() => {
  vi.restoreAllMocks();
});

describe('linkedRangesFilter', () => {
  it('deletes a matching close marker in the original coordinate space', () => {
    vi.spyOn(Document.prototype, 'pairs').mockReturnValue([
      {
        kind: 'ruby',
        open: { start: 0, end: 3 },
        close: { start: 4, end: 7 },
      },
    ]);
    const state = EditorState.create({
      doc: '《x》',
      extensions: [parserStateField, linkedRangesFilter],
    });

    const transaction = state.update({ changes: { from: 0, to: 1 } });

    expect(transaction.newDoc.toString()).toBe('x');
  });
});

describe('halfToFullWidthFilter', () => {
  it('preserves every cursor after a multi-selection rewrite', () => {
    const state = EditorState.create({
      doc: 'ab',
      selection: EditorSelection.create([
        EditorSelection.cursor(0),
        EditorSelection.cursor(2),
      ]),
      extensions: [
        EditorState.allowMultipleSelections.of(true),
        halfToFullWidthFilter,
      ],
    });
    const transaction = state.update({
      changes: [
        { from: 0, insert: '[' },
        { from: 2, insert: '[' },
      ],
      selection: EditorSelection.create(
        [EditorSelection.cursor(1), EditorSelection.cursor(4)],
        1,
      ),
    });

    expect(transaction.newDoc.toString()).toBe('［＃］ab［＃］');
    expect(transaction.newSelection.ranges.map((range) => range.head)).toEqual([
      2, 7,
    ]);
    expect(transaction.newSelection.mainIndex).toBe(1);
  });
});
