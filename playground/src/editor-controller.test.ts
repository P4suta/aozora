import { undo, undoDepth } from '@codemirror/commands';
import { EditorView } from '@codemirror/view';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { createEditor } from './editor-controller';

let controller: ReturnType<typeof createEditor> | null = null;

afterEach(() => {
  controller?.destroy();
  controller = null;
});

describe('editor controller', () => {
  it('does not add externally restored source to undo history', () => {
    const onChange = vi.fn();
    const parent = document.createElement('div');
    document.body.append(parent);
    controller = createEditor(parent, 'initial sample', onChange);
    const content = parent.querySelector('.cm-content');
    if (!(content instanceof HTMLElement)) {
      throw new Error('CodeMirror content was not created');
    }
    const view = EditorView.findFromDOM(content);
    if (!view) throw new Error('CodeMirror view was not found');

    controller.setValue('restored draft');

    expect(view.state.doc.toString()).toBe('restored draft');
    expect(undoDepth(view.state)).toBe(0);
    expect(undo(view)).toBe(false);
    expect(view.state.doc.toString()).toBe('restored draft');
    expect(onChange).not.toHaveBeenCalled();
    parent.remove();
  });
});
