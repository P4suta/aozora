import { Annotation, EditorSelection, EditorState, type ChangeSpec } from '@codemirror/state';

/**
 * 半角 ASCII 1 文字を打った時の自動置換テーブル。aozora 流の
 * 「打ったら記法らしい形にまで一気に整える」挙動。
 *
 * - 開き括弧（`[`, `<`, `{`）は **対応する閉じ括弧も同時に挿入** し、
 *   カーソルを内側に置く。`[` だけは slug 注記の頻度が圧倒的なので
 *   `＃` まで一気に入れる
 * - 閉じ括弧（`]`, `>`, `}`）は 1 文字だけ全角化（カーソルは後ろ）
 * - `|` / `*` は対応する閉じが無いので 1 文字置換（カーソルは後ろ）
 *
 * `(` / `)` は地の文の半角丸括弧として残すケースが多い（青空文庫
 * ソース内でも頻出）ので変換対象外。リポジトリ内 LSP の
 * `crates/aozora-cli/src/lsp/on_type_formatting.rs` とも揃えている。
 */
interface ReplacementSpec {
  /** 挿入文字列（半角 1 文字の置換先）。 */
  insert: string;
  /** 挿入後にカーソルを置く位置（insert の先頭からの UTF-16 オフセット）。 */
  cursorOffset: number;
}

const HALF_TO_FULL: Record<string, ReplacementSpec> = {
  // 開き → ペアで挿入、カーソルは内側
  '[': { insert: '［＃］', cursorOffset: 2 }, // ［＃|］ で slug 注記の本体に直行
  '<': { insert: '《》', cursorOffset: 1 }, // 《|》 ruby reading
  '{': { insert: '〔〕', cursorOffset: 1 }, // 〔|〕 アクセント分解
  // 閉じ → 単体置換、カーソルは後ろ
  ']': { insert: '］', cursorOffset: 1 },
  '>': { insert: '》', cursorOffset: 1 },
  '}': { insert: '〕', cursorOffset: 1 },
  // 対応の無いマーカー → 単体置換、カーソルは後ろ
  '|': { insert: '｜', cursorOffset: 1 }, // ｜ ruby base 開始
  '*': { insert: '※', cursorOffset: 1 }, // ※ gaiji マーカー
  '#': { insert: '＃', cursorOffset: 1 }, // ＃ slug マーカー（［＃...］ 内で常用）
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
    // fromB は変換対象の半角 1 文字の「新 doc 上の開始位置」。
    // 同じ範囲を全角の置換結果で上書きする。
    replacements.push({ from: fromB, to: fromB + text.length, insert: spec.insert });
    cursorAfter = fromB + spec.cursorOffset;
  });

  if (replacements.length === 0) return tr;
  return [
    tr,
    {
      changes: replacements,
      selection: cursorAfter !== null ? EditorSelection.cursor(cursorAfter) : undefined,
      annotations: ON_TYPE.of(true),
      sequential: true,
    },
  ];
});
