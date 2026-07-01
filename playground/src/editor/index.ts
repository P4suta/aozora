import { Annotation, Compartment, EditorState } from '@codemirror/state';
import {
  EditorView,
  keymap,
  drawSelection,
  highlightActiveLine,
  highlightSpecialChars,
  lineNumbers,
  placeholder,
  rectangularSelection,
} from '@codemirror/view';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { bracketMatching, foldGutter, foldKeymap } from '@codemirror/language';
import { searchKeymap } from '@codemirror/search';
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete';
import { parserStateField, setParseCallbacks, type ParserState } from './parserState';
import { aozoraDecorations } from './decorations';
import { aozoraTheme } from './theme';
import { aozoraLinter, aozoraLintGutter } from './linter';
import { aozoraCompletion } from './completion';
import { aozoraHover } from './hover';
import { linkedRangesFilter } from './linkedRanges';
import { aozoraFolding } from './folding';
import { halfToFullWidthFilter } from './onType';
import { aozoraWrapKeymap } from './wrapCommands';
import { aozoraInlayHints } from './inlayHints';
import { t } from '../i18n';

export interface AozoraEditorOptions {
  parent: HTMLElement;
  initialValue: string;
  onChange?: (next: string) => void;
  onParse?: (payload: ParserState) => void;
  /** Open the command palette (bound to Mod-Shift-p). */
  onOpenPalette?: () => void;
}

/**
 * Compartments let runtime UI toggles (settings panel) reconfigure
 * specific extensions without re-creating the EditorView. We expose
 * them by name so the UI layer can call
 * `view.dispatch({ effects: halfWidthCompartment.reconfigure(...) })`.
 */
export const halfWidthCompartment = new Compartment();
export const inlayHintsCompartment = new Compartment();

/**
 * Tag transactions that come from `Editor.tsx`'s external setValue
 * effect (e.g. SampleLoader, share-URL restore) so `onChange` does
 * not echo them back to the parent and create a feedback loop.
 */
export const externalUpdate = Annotation.define<true>();

/**
 * 自動括弧閉じの対象セット。`closeBrackets()` のデフォルトは ASCII
 * `() [] {} '' "" ``` のみなので、aozora 用に全角括弧を上書きする。
 *
 * 入力に対するクローズ：
 *   《 → 《》       「 → 「」      〔 → 〔〕
 *   （ → （）      ［ → ［］
 */
const aozoraCloseBracketsConfig = EditorState.languageData.of(() => [
  {
    closeBrackets: {
      brackets: ['《', '「', '〔', '（', '［'],
    },
  },
]);

/**
 * Build a CodeMirror 6 editor for Aozora notation. The configuration
 * is intentionally split into one extension array so that subsequent
 * features (syntax decorations, linter, completion, hover, etc.) can
 * be added in a single place as Phase 2 progresses.
 */
export function createAozoraEditor(options: AozoraEditorOptions): EditorView {
  setParseCallbacks({ onParse: options.onParse });

  const state = EditorState.create({
    doc: options.initialValue,
    extensions: [
      lineNumbers(),
      highlightActiveLine(),
      highlightSpecialChars(),
      drawSelection(),
      rectangularSelection(),
      history(),
      foldGutter(),
      bracketMatching(),
      EditorState.allowMultipleSelections.of(true),
      EditorState.tabSize.of(2),
      EditorView.lineWrapping,
      placeholder(t('editorPlaceholder')),
      closeBrackets(),
      aozoraCloseBracketsConfig,
      parserStateField,
      aozoraTheme,
      aozoraDecorations,
      aozoraLintGutter,
      aozoraLinter,
      aozoraCompletion,
      aozoraHover,
      linkedRangesFilter,
      halfWidthCompartment.of(halfToFullWidthFilter),
      aozoraFolding,
      inlayHintsCompartment.of(aozoraInlayHints),
      keymap.of([
        ...aozoraWrapKeymap,
        {
          // Open the command palette. Mod-Shift-p avoids the browser's
          // Mod-p (print) and CM6's own bindings.
          key: 'Mod-Shift-p',
          run: () => {
            options.onOpenPalette?.();
            return true;
          },
          preventDefault: true,
        },
        ...closeBracketsKeymap,
        // 注意：indentWithTab は **入れない**。
        //   - 青空文庫記法では tab インデントは使わず全角スペースで字下げするのが流儀
        //   - tab を奪うと、`｜` トリガーで出た補完候補（ruby snippet 等）の
        //     Tab accept や、スニペット展開後のタブストップ送り（${1} → ${2}）が
        //     横取りされてしまう（autocompletion の defaultKeymap が動かなくなる）
        // 同じ理由で <code>indentMore / indentLess</code> も入れない。
        ...defaultKeymap,
        ...historyKeymap,
        ...foldKeymap,
        ...searchKeymap,
      ]),
      EditorView.updateListener.of((update) => {
        if (!update.docChanged) return;
        // Skip if any of the contributing transactions was marked as
        // external — that's how `Editor.tsx` mirrors `props.value`
        // into the editor without bouncing back through `onChange`.
        if (update.transactions.some((tr) => tr.annotation(externalUpdate))) return;
        options.onChange?.(update.state.doc.toString());
      }),
    ],
  });

  return new EditorView({ state, parent: options.parent });
}

export type {
  ParserState,
  HeadingEntry,
  ContainerFold,
  NodeEntry,
  DiagnosticEntry,
  PairEntry,
  GaijiResolutionEntry,
  ProfilePhaseEntry,
} from './parserState';
export { parserStateField, utf16ToByte, byteToUtf16 } from './parserState';
export { WRAP_PALETTE, getWrapCommand, WRAP_SHAPES } from './wrapCommands';
