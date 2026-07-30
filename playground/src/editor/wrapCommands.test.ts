import { EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { afterEach, describe, expect, it } from 'vitest';

import { getWrapCommand } from './wrapCommands';

let view: EditorView | null = null;

afterEach(() => {
  view?.destroy();
  view = null;
});

describe('selection wrapping', () => {
  it('preserves marker-looking text in the selected source', () => {
    const selected = '${0} BASE';
    view = new EditorView({
      parent: document.body,
      state: EditorState.create({
        doc: selected,
        selection: { anchor: 0, head: selected.length },
      }),
    });

    expect(getWrapCommand('aozora.wrap.ruby')?.(view)).toBe(true);
    expect(view.state.doc.toString()).toBe('｜${0} BASE《》');
    expect(view.state.selection.main.anchor).toBe('｜${0} BASE《'.length);
  });

  it('returns no command for an unknown identifier', () => {
    expect(getWrapCommand('unknown')).toBeNull();
  });
});
