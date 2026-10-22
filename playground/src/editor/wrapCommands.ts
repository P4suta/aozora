import type { Command, EditorView, KeyBinding } from '@codemirror/view';

export interface WrapShape {
  id: string;
  /** Uses `BASE` for the selection and `${0}` for the final cursor. */
  template: string;
}

export const WRAP_SHAPES: readonly WrapShape[] = [
  { id: 'aozora.wrap.ruby', template: '｜BASE《${0}》' },
  {
    id: 'aozora.wrap.angleQuote',
    template: '≪BASE≫${0}',
  },
  {
    id: 'aozora.wrap.bouten',
    template: 'BASE［＃「BASE」に傍点］${0}',
  },
  {
    id: 'aozora.wrap.kagikakko',
    template: '「BASE」${0}',
  },
  {
    id: 'aozora.wrap.kikkou',
    template: '〔BASE〕${0}',
  },
  {
    id: 'aozora.wrap.chuki',
    template: '［＃BASE］${0}',
  },
] as const;

/** Empty selections substitute an empty `BASE` and retain the `${0}` cursor position. */
export function wrapCommand(shape: WrapShape): Command {
  return (view: EditorView) => {
    const sel = view.state.selection.main;
    const selected = view.state.sliceDoc(sel.from, sel.to);
    const marker = '${0}';
    const markerOffset = shape.template.indexOf(marker);
    const beforeMarker =
      markerOffset < 0 ? shape.template : shape.template.slice(0, markerOffset);
    const afterMarker =
      markerOffset < 0
        ? ''
        : shape.template.slice(markerOffset + marker.length);
    const beforeCursor = beforeMarker.split('BASE').join(selected);
    const replacement = beforeCursor + afterMarker.split('BASE').join(selected);
    view.dispatch({
      changes: { from: sel.from, to: sel.to, insert: replacement },
      selection: {
        anchor:
          sel.from +
          (markerOffset < 0 ? replacement.length : beforeCursor.length),
      },
      scrollIntoView: true,
    });
    return true;
  };
}

const SHAPE_BY_ID: Record<string, WrapShape> = Object.fromEntries(
  WRAP_SHAPES.map((s) => [s.id, s]),
);

export function getWrapCommand(id: string): Command | null {
  const shape = SHAPE_BY_ID[id];
  return shape ? wrapCommand(shape) : null;
}

export const aozoraWrapKeymap: KeyBinding[] = [
  {
    key: 'Mod-Alt-r',
    run: wrapCommand(SHAPE_BY_ID['aozora.wrap.ruby']!),
    preventDefault: true,
  },
  {
    key: 'Mod-Alt-Shift-r',
    run: wrapCommand(SHAPE_BY_ID['aozora.wrap.angleQuote']!),
    preventDefault: true,
  },
  {
    key: 'Mod-Alt-b',
    run: wrapCommand(SHAPE_BY_ID['aozora.wrap.bouten']!),
    preventDefault: true,
  },
];
