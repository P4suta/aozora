import { snippet } from '@codemirror/autocomplete';
import { EditorView, type Command, type KeyBinding } from '@codemirror/view';

export interface WrapShape {
  /** Stable id, matches aozora-tools VSCode command names. */
  id: string;
  /** Snippet template with `BASE` for the selection and `${0}` for the final cursor. */
  template: string;
  /** Short Japanese label for command palette / menu surfaces. */
  description: string;
}

/**
 * 6 selection-wrap actions, ported from
 * `~/projects/aozora-tools/editors/vscode/src/wrap.ts`. The shapes
 * (and the choice to always emit a leading `｜` for ruby) are kept
 * identical so users alternating between the VSCode extension and
 * the web playground get the same muscle memory.
 */
export const WRAP_SHAPES: readonly WrapShape[] = [
  { id: 'aozora.wrap.ruby', template: '｜BASE《${0}》', description: 'ルビ' },
  { id: 'aozora.wrap.angleQuote', template: '≪BASE≫${0}', description: '二重山括弧' },
  { id: 'aozora.wrap.bouten', template: 'BASE［＃「BASE」に傍点］${0}', description: '傍点' },
  { id: 'aozora.wrap.kagikakko', template: '「BASE」${0}', description: '鉤括弧で囲む' },
  { id: 'aozora.wrap.kikkou', template: '〔BASE〕${0}', description: '亀甲括弧で囲む' },
  { id: 'aozora.wrap.chuki', template: '［＃BASE］${0}', description: '注記で囲む' },
] as const;

function escapeSnippet(text: string): string {
  return text.replace(/\\/g, '\\\\').replace(/\$/g, '\\$').replace(/\}/g, '\\}');
}

/**
 * Build a CM6 `Command` that applies the given wrap template to the
 * current selection. If the selection is empty, `BASE` substitutes
 * the empty string — the resulting snippet still has its `${0}`
 * tabstop, so cursor placement is well-defined.
 */
export function wrapCommand(shape: WrapShape): Command {
  return (view: EditorView) => {
    const sel = view.state.selection.main;
    const selected = view.state.sliceDoc(sel.from, sel.to);
    const body = shape.template.split('BASE').join(escapeSnippet(selected));
    const insert = snippet(body);
    // CM6 の snippet() が返す関数は (view, completion, from, to) を取り、
    // 内部的に completion は autocomplete 経由の文脈情報を渡すためだけに
    // 使われる。keymap から直接呼ぶ場合は completion 情報が無く、また
    // snippet 展開そのものはこの引数を使わないので `null` で問題ない。
    // 型は `Completion` 必須なので `null as never` でアサート。
    insert(view, null as never, sel.from, sel.to);
    return true;
  };
}

const SHAPE_BY_ID: Record<string, WrapShape> = Object.fromEntries(
  WRAP_SHAPES.map((s) => [s.id, s]),
);

/**
 * Resolved command palette entries — surfaced to the playground UI
 * so users can invoke wrap actions whose keybindings (e.g. the
 * full-width brackets) are not typeable.
 */
export const WRAP_PALETTE: ReadonlyArray<{ id: string; description: string }> = WRAP_SHAPES.map(
  (s) => ({ id: s.id, description: s.description }),
);

export function getWrapCommand(id: string): Command | null {
  const shape = SHAPE_BY_ID[id];
  return shape ? wrapCommand(shape) : null;
}

/**
 * Keybindings registered globally. Mirrors aozora-tools' VSCode
 * bindings: Ctrl/Cmd+Alt+R for ruby, Ctrl/Cmd+Alt+B for bouten.
 * angleQuote is on Shift+Ctrl/Cmd+Alt+R (aozora-tools leaves it
 * unbound; we pick the natural extension since the playground has
 * no command palette plumbing yet).
 */
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
