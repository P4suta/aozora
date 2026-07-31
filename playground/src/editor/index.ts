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

export interface AozoraEditorOptions {
  parent: HTMLElement;
  initialValue: string;
  onChange?: (next: string) => void;
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
export const engineFeaturesCompartment = new Compartment();
export const localeCompartment = new Compartment();

/**
 * Tag transactions that come from `EditorController.setValue`
 * (e.g. sample or share-URL restoration) so `onChange` does
 * not echo them back to the parent and create a feedback loop.
 */
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

/**
 * Build a CodeMirror 6 editor for Aozora notation. The configuration
 * is intentionally split into one extension array so that subsequent
 * features (syntax decorations, linter, completion, hover, etc.) can
 * be added in a single place as Phase 2 progresses.
 */
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
      EditorState.tabSize.of(2),
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
        // Tab must remain available to completion and snippet keymaps.
        ...defaultKeymap,
        ...historyKeymap,
        ...foldKeymap,
        ...searchKeymap,
      ]),
      EditorView.updateListener.of((update) => {
        if (!update.docChanged) return;
        // Skip if any of the contributing transactions was marked as
        // external — that's how the controller mirrors application state
        // into the editor without bouncing back through `onChange`.
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
