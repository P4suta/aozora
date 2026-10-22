import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { bracketMatching, foldGutter, foldKeymap } from '@codemirror/language';
import { searchKeymap } from '@codemirror/search';
import {
  Annotation,
  Compartment,
  EditorState,
  type Extension,
} from '@codemirror/state';
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  placeholder,
  rectangularSelection,
} from '@codemirror/view';
import { t } from '../i18n';
import { aozoraCompletion } from './completion';
import { aozoraDecorations } from './decorations';
import { aozoraFolding } from './folding';
import { aozoraHover } from './hover';
import { aozoraInlayHints } from './inlayHints';
import { linkedRangesFilter } from './linkedRanges';
import { aozoraLinter, aozoraLintGutter } from './linter';
import { halfToFullWidthFilter } from './onType';
import { parserStateField } from './parserState';
import { aozoraTheme } from './theme';
import { aozoraWrapKeymap } from './wrapCommands';

const EDITOR_TAB_SIZE = 2;

export interface AozoraEditorOptions {
  parent: HTMLElement;
  initialValue: string;
  onChange?: (next: string) => void;
  /** Open the command palette with `Mod-Shift-p`. */
  onOpenPalette?: () => void;
}

/** Reconfigure half-width conversion without recreating the editor. */
export const halfWidthCompartment = new Compartment();
export const inlayHintsCompartment = new Compartment();
export const engineFeaturesCompartment = new Compartment();
export const localeCompartment = new Compartment();

/** Mark external transactions to suppress the editor's `onChange` callback. */
export const externalUpdate = Annotation.define<true>();

const aozoraCloseBracketsConfig = EditorState.languageData.of(() => [
  {
    closeBrackets: {
      brackets: ['《', '「', '〔', '（', '［'],
    },
  },
]);

export function aozoraEngineExtensions(inlayHintsEnabled: boolean): Extension {
  return [
    parserStateField,
    aozoraDecorations,
    aozoraLintGutter,
    aozoraLinter,
    aozoraCompletion,
    aozoraHover,
    linkedRangesFilter,
    aozoraFolding,
    inlayHintsCompartment.of(inlayHintsEnabled ? aozoraInlayHints : []),
  ];
}

export function aozoraLocaleExtensions(): Extension {
  return [
    EditorView.contentAttributes.of({
      'aria-label': t('editorPaneTitle'),
    }),
    placeholder(t('editorPlaceholder')),
  ];
}

/** Build a CodeMirror editor for Aozora notation. */
export function createAozoraEditor(options: AozoraEditorOptions): EditorView {
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
      EditorState.tabSize.of(EDITOR_TAB_SIZE),
      EditorView.lineWrapping,
      localeCompartment.of(aozoraLocaleExtensions()),
      closeBrackets(),
      aozoraCloseBracketsConfig,
      aozoraTheme,
      halfWidthCompartment.of(halfToFullWidthFilter),
      engineFeaturesCompartment.of([]),
      keymap.of([
        ...aozoraWrapKeymap,
        {
          key: 'Mod-Shift-p',
          run: () => {
            options.onOpenPalette?.();
            return true;
          },
          preventDefault: true,
        },
        ...closeBracketsKeymap,
        ...defaultKeymap,
        ...historyKeymap,
        ...foldKeymap,
        ...searchKeymap,
      ]),
      EditorView.updateListener.of((update) => {
        if (!update.docChanged) return;
        if (update.transactions.some((tr) => tr.annotation(externalUpdate)))
          return;
        options.onChange?.(update.state.doc.toString());
      }),
    ],
  });

  return new EditorView({ state, parent: options.parent });
}

export type {
  ContainerFold,
  DiagnosticEntry,
  GaijiResolutionEntry,
  NodeEntry,
  PairEntry,
  ParserState,
} from './parserState';
export { byteToUtf16, parserStateField, utf16ToByte } from './parserState';
export { getWrapCommand, WRAP_SHAPES } from './wrapCommands';
