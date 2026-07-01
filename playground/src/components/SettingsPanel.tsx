import { createSignal, For, onCleanup, Show } from 'solid-js';
import { EditorView } from '@codemirror/view';
import { halfWidthCompartment, inlayHintsCompartment } from '../editor';
import { halfToFullWidthFilter } from '../editor/onType';
import { aozoraInlayHints } from '../editor/inlayHints';
import { clearStoredSource } from '../storage';
import { applyTheme, loadThemePref, saveThemePref, type ThemePref } from '../theme';
import { locale, setLocale, t, type Locale } from '../i18n';

interface SettingsPanelProps {
  view: EditorView | null;
  onResetStorage?: () => void;
}

const THEME_CHOICES: ReadonlyArray<{ value: ThemePref; label: string }> = [
  { value: 'auto', label: 'Auto' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

const LANG_CHOICES: ReadonlyArray<{ value: Locale; label: string }> = [
  { value: 'ja', label: '日本語' },
  { value: 'en', label: 'English' },
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

  function pickLang(next: Locale) {
    setLocale(next);
  }

  return (
    <div class="settings-panel-root" ref={rootEl}>
      <button
        type="button"
        class="settings-trigger"
        onClick={() => setOpen((v) => !v)}
        title={t('settingsTrigger')}
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
              {t('settingHalfWidth')}
              <span class="settings-sub">{t('settingHalfWidthSub')}</span>
            </span>
          </label>
          <label class="settings-row">
            <input type="checkbox" checked={inlay()} onChange={toggleInlay} />
            <span class="settings-label">
              {t('settingInlay')}
              <span class="settings-sub">{t('settingInlaySub')}</span>
            </span>
          </label>
          <div class="settings-divider" />
          <div class="settings-row settings-row-radio">
            <span class="settings-label">
              {t('settingTheme')}
              <span class="settings-sub">{t('settingThemeSub')}</span>
            </span>
            <div class="settings-radio-group" role="radiogroup" aria-label={t('settingTheme')}>
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
          <div class="settings-row settings-row-radio">
            <span class="settings-label">
              {t('settingLanguage')}
              <span class="settings-sub">{t('settingLanguageSub')}</span>
            </span>
            <div class="settings-radio-group" role="radiogroup" aria-label={t('settingLanguage')}>
              <For each={LANG_CHOICES}>
                {(choice) => (
                  <label class="settings-radio">
                    <input
                      type="radio"
                      name="lang-pref"
                      value={choice.value}
                      checked={locale() === choice.value}
                      onChange={() => pickLang(choice.value)}
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
            {t('settingReset')}
            <span class="settings-sub">{t('settingResetSub')}</span>
          </button>
        </div>
      </Show>
    </div>
  );
}
