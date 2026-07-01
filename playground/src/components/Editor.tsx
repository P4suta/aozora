import { createEffect, onCleanup, onMount, on } from 'solid-js';
import { EditorView } from '@codemirror/view';
import { createAozoraEditor, externalUpdate, type ParserState } from '../editor';
import { ensureWasmReady } from '../wasm-loader';
import { error as logError } from '../logger';
import { t } from '../i18n';

interface EditorProps {
  value: string;
  onInput: (next: string) => void;
  onParse?: (payload: ParserState) => void;
  onReady?: (view: EditorView) => void;
  onOpenPalette?: () => void;
}

export default function Editor(props: EditorProps) {
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  let mounted = false;

  onMount(async () => {
    try {
      await ensureWasmReady();
    } catch (err) {
      logError('WASM init failed before editor mount:', err);
      // WASM が無い状態で CM6 を作っても parserState が空のままで
      // 何もできないので、host に静的なメッセージだけ出して abort。
      // 重要：App.tsx 側の error-banner（再読み込みボタン付き）が並んで
      // 出ているので、ユーザーへの行動指示はそちらに任せる。
      host.innerHTML = `<div class="editor-disabled-placeholder">${t('editorDisabled')}</div>`;
      return;
    }
    view = createAozoraEditor({
      parent: host,
      initialValue: props.value,
      onChange: (next) => props.onInput(next),
      onParse: (payload) => props.onParse?.(payload),
      onOpenPalette: () => props.onOpenPalette?.(),
    });
    mounted = true;
    props.onReady?.(view);
  });

  createEffect(
    on(
      () => props.value,
      (next) => {
        if (!view || !mounted) return;
        const current = view.state.doc.toString();
        if (current === next) return;
        // The `externalUpdate` annotation marks this transaction as
        // "originated from the parent setSource", so the editor's
        // updateListener will not echo it back through `onInput`.
        view.dispatch({
          changes: { from: 0, to: current.length, insert: next },
          annotations: externalUpdate.of(true),
        });
      },
      { defer: true },
    ),
  );

  onCleanup(() => view?.destroy());

  return <div ref={host} class="editor cm-host" />;
}
