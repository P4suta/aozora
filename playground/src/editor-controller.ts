import type { EditorController, TextRange } from '@aozora/playground-ui';
import { forceLinting } from '@codemirror/lint';
import { Transaction } from '@codemirror/state';
import { EditorView } from '@codemirror/view';

import {
  aozoraEngineExtensions,
  aozoraLocaleExtensions,
  createAozoraEditor,
  engineFeaturesCompartment,
  externalUpdate,
  getWrapCommand,
  halfWidthCompartment,
  inlayHintsCompartment,
  localeCompartment,
  parserStateField,
} from './editor';
import { aozoraInlayHints } from './editor/inlayHints';
import { halfToFullWidthFilter } from './editor/onType';

export interface EngineAwareEditorController extends EditorController {
  enableEngineFeatures(): void;
  refreshLocale(): void;
}

export function createEditor(
  parent: HTMLElement,
  initialValue: string,
  onChange: (value: string) => void,
  engineReady = false,
): EngineAwareEditorController {
  const view = createAozoraEditor({
    parent,
    initialValue,
    onChange,
  });
  let engineFeaturesEnabled = false;
  let inlayHintsEnabled = true;

  const enableEngineFeatures = () => {
    if (engineFeaturesEnabled) return;
    view.dispatch({
      effects: engineFeaturesCompartment.reconfigure(
        aozoraEngineExtensions(inlayHintsEnabled),
      ),
    });
    engineFeaturesEnabled = true;
  };
  if (engineReady) enableEngineFeatures();

  return {
    enableEngineFeatures,
    refreshLocale() {
      view.dispatch({
        effects: localeCompartment.reconfigure(aozoraLocaleExtensions()),
      });
      forceLinting(view);
    },
    setValue(value: string) {
      const current = view.state.doc.toString();
      if (current === value) return;
      view.dispatch({
        changes: { from: 0, to: current.length, insert: value },
        annotations: [
          externalUpdate.of(true),
          Transaction.addToHistory.of(false),
        ],
      });
    },
    focus: () => view.focus(),
    revealRange(range: TextRange) {
      const from = Math.max(0, Math.min(range.start, view.state.doc.length));
      const to = Math.max(from, Math.min(range.end, view.state.doc.length));
      view.dispatch({
        selection: { anchor: from, head: to },
        effects: EditorView.scrollIntoView(from, { y: 'center' }),
      });
    },
    runCommand(commandId: string) {
      return getWrapCommand(commandId)?.(view) ?? false;
    },
    setSetting(settingId: string, enabled: boolean) {
      if (settingId === 'halfWidthConversion') {
        view.dispatch({
          effects: halfWidthCompartment.reconfigure(
            enabled ? halfToFullWidthFilter : [],
          ),
        });
      } else if (settingId === 'gaijiInlayHints') {
        inlayHintsEnabled = enabled;
        if (!engineFeaturesEnabled) return;
        view.dispatch({
          effects: inlayHintsCompartment.reconfigure(
            enabled ? aozoraInlayHints : [],
          ),
        });
      }
    },
    destroy() {
      view.state.field(parserStateField, false)?.doc?.free();
      view.destroy();
    },
  };
}
