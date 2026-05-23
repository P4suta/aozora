import { createSignal, For, onCleanup, Show } from 'solid-js';
import { EditorView } from '@codemirror/view';
import { halfWidthCompartment, inlayHintsCompartment } from '../editor';
import { halfToFullWidthFilter } from '../editor/onType';
import { aozoraInlayHints } from '../editor/inlayHints';
import { clearStoredSource } from '../storage';
import { applyTheme, loadThemePref, saveThemePref, type ThemePref } from '../theme';

interface SettingsPanelProps {
  view: EditorView | null;
  onResetStorage?: () => void;
}

const THEME_CHOICES: ReadonlyArray<{ value: ThemePref; label: string }> = [
  { value: 'auto', label: 'Auto' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

export default function SettingsPanel(props: SettingsPanelProps) {
  const [open, setOpen] = createSignal(false);
  const [halfWidth, setHalfWidth] = createSignal(true);
  const [inlay, setInlay] = createSignal(true);
  const [theme, setTheme] = createSignal<ThemePref>(loadThemePref());

  let rootEl: HTMLDivElement | undefined;

  function handleClickOutside(event: MouseEvent) {
    if (!open()) return;
    if (rootEl && !rootEl.contains(event.target as Node)) {
      setOpen(false);
    }
  }
  document.addEventListener('mousedown', handleClickOutside);
  onCleanup(() => document.removeEventListener('mousedown', handleClickOutside));

  function toggleHalfWidth() {
    const next = !halfWidth();
    setHalfWidth(next);
    props.view?.dispatch({
      effects: halfWidthCompartment.reconfigure(next ? halfToFullWidthFilter : []),
    });
  }

  function toggleInlay() {
    const next = !inlay();
    setInlay(next);
    props.view?.dispatch({
      effects: inlayHintsCompartment.reconfigure(next ? aozoraInlayHints : []),
    });
  }

  function pickTheme(pref: ThemePref) {
    setTheme(pref);
    saveThemePref(pref);
    applyTheme(pref);
  }

  return (
    <div class="settings-panel-root" ref={rootEl}>
      <button
        type="button"
        class="settings-trigger"
        onClick={() => setOpen((v) => !v)}
        title="エディタ設定"
        aria-haspopup="true"
        aria-expanded={open()}
      >
        ⚙
      </button>
      <Show when={open()}>
        <div class="settings-popover" role="menu">
          <label class="settings-row">
            <input type="checkbox" checked={halfWidth()} onChange={toggleHalfWidth} />
            <span class="settings-label">
              半角→全角の即時変換
              <span class="settings-sub">[ → ［ 、 | → ｜ など 8 種</span>
            </span>
          </label>
          <label class="settings-row">
            <input type="checkbox" checked={inlay()} onChange={toggleInlay} />
            <span class="settings-label">
              外字インレイヒント
              <span class="settings-sub">※［＃...］の後ろに →解決字 を表示</span>
            </span>
          </label>
          <div class="settings-divider" />
          <div class="settings-row settings-row-radio">
            <span class="settings-label">
              テーマ
              <span class="settings-sub">Auto は OS 設定に追従</span>
            </span>
            <div class="settings-radio-group" role="radiogroup" aria-label="テーマ">
              <For each={THEME_CHOICES}>
                {(choice) => (
                  <label class="settings-radio">
                    <input
                      type="radio"
                      name="theme-pref"
                      value={choice.value}
                      checked={theme() === choice.value}
                      onChange={() => pickTheme(choice.value)}
                    />
                    <span>{choice.label}</span>
                  </label>
                )}
              </For>
            </div>
          </div>
          <div class="settings-divider" />
          <button
            type="button"
            class="settings-action"
            onClick={() => {
              clearStoredSource();
              props.onResetStorage?.();
              setOpen(false);
            }}
          >
            保存をリセット
            <span class="settings-sub">localStorage の編集内容を消去（共有 URL は影響なし）</span>
          </button>
        </div>
      </Show>
    </div>
  );
}
