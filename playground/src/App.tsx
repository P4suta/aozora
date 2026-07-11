import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import { EditorView } from '@codemirror/view';
import Editor from './components/Editor';
import PreviewPane from './components/PreviewPane';
import SampleLoader from './components/SampleLoader';
import PerfBadge from './components/PerfBadge';
import { ensureWasmReady, version as wasmParserVersion } from './wasm-loader';
import type { HeadingEntry, ParserState, ProfilePhaseEntry } from './editor';
import { buildShareUrl, readShareTextFromUrl, syncTextToUrl } from './share';
import { clearStoredSource, loadStoredSource, saveSource } from './storage';
import { SAMPLES, DEFAULT_SAMPLE_ID } from './samples';
import NotationGuide from './components/NotationGuide';
import CommandPalette from './components/CommandPalette';
import SettingsPanel from './components/SettingsPanel';
import { error as logError } from './logger';
import { t, tf } from './i18n';

const EMPTY_ENVELOPE = '{"schemaVersion":2,"data":[]}';

interface ParsePayload {
  html: string;
  serialized: string;
  diagJson: string;
  nodesJson: string;
  headings: HeadingEntry[];
  profile: ProfilePhaseEntry[];
  parseDurationMs: number;
  byteLen: number;
}

const EMPTY_PAYLOAD: ParsePayload = {
  html: '',
  serialized: '',
  diagJson: EMPTY_ENVELOPE,
  nodesJson: EMPTY_ENVELOPE,
  headings: [],
  profile: [],
  parseDurationMs: 0,
  byteLen: 0,
};

const DEFAULT_TEXT =
  SAMPLES.find((s) => s.id === DEFAULT_SAMPLE_ID)?.text ?? SAMPLES[0]!.text;

export default function App() {
  // 起動時の source の優先順位:
  //   1. `?text=` 共有 URL（友人から送られたリンクを最優先）
  //   2. localStorage（前回の編集を復元）
  //   3. デフォルトサンプル
  // 1 と 2 は別レイヤなので、URL 由来で開いてもその後の編集は
  // localStorage に保存され続ける（次回 URL なしで開けば続きが見える）。
  const urlRead = readShareTextFromUrl();
  const initialFromStorage = urlRead.status === 'ok' ? null : loadStoredSource();
  const initialText =
    urlRead.status === 'ok' ? urlRead.text : (initialFromStorage ?? DEFAULT_TEXT);

  const [source, setSource] = createSignal(initialText);
  const [wasmReady, setWasmReady] = createSignal(false);
  const [wasmError, setWasmError] = createSignal<string | null>(null);
  // Parser build version for the footer — read from the wasm engine once it
  // initialises (aozora-buildstamp is the single authority; no hard-coded
  // literal). Null until the engine resolves.
  const [wasmVersion, setWasmVersion] = createSignal<string | null>(null);
  const [toast, setToast] = createSignal<string | null>(null);
  const [parsePayload, setParsePayload] = createSignal<ParsePayload>(EMPTY_PAYLOAD);
  const [editorView, setEditorView] = createSignal<EditorView | null>(null);
  const [showGuide, setShowGuide] = createSignal(false);
  const [paletteOpen, setPaletteOpen] = createSignal(false);
  // Layout mode for the editor / preview panes. Useful on phones
  // (one-pane focus) but also on desktop when the user wants a
  // wider editor or full-screen preview.
  const [layoutMode, setLayoutMode] = createSignal<'split' | 'editor' | 'preview'>('split');

  onMount(() => {
    ensureWasmReady()
      .then(() => {
        setWasmReady(true);
        setWasmVersion(wasmParserVersion());
      })
      .catch((err: unknown) => {
        logError('WASM init failed:', err);
        setWasmError(err instanceof Error ? err.message : String(err));
      });
    if (urlRead.status === 'invalid') {
      // 起動時の URL デコード失敗を通知。setSource は走らないので default に落ちている。
      flashToast(t('toastShareDecodeFail'));
    }
  });

  // URL の `?text=` 同期は history.replaceState の rate limit を避けるため 300ms debounce。
  // parse 自体はリアルタイム（CM6 の StateField がキー入力ごとに走る）。
  let urlSyncHandle: number | undefined;
  // localStorage は容量が大きく書き込みコストも軽いが、毎キー打鍵だと
  // 数十 KB×数千回 / 秒は無駄なので 500ms debounce する。
  let storageSaveHandle: number | undefined;
  // 保存失敗を 1 セッションで何度も通知しないためのラッチ。
  let storageFailNotified = false;
  createEffect(
    on(source, (text) => {
      if (urlSyncHandle !== undefined) clearTimeout(urlSyncHandle);
      urlSyncHandle = window.setTimeout(() => syncTextToUrl(text), 300);
      if (storageSaveHandle !== undefined) clearTimeout(storageSaveHandle);
      storageSaveHandle = window.setTimeout(() => {
        const ok = saveSource(text);
        if (!ok && !storageFailNotified) {
          storageFailNotified = true;
          flashToast(t('toastSaveFail'));
        }
        if (ok) storageFailNotified = false;
      }, 500);
    }),
  );

  onCleanup(() => {
    if (urlSyncHandle !== undefined) clearTimeout(urlSyncHandle);
    if (storageSaveHandle !== undefined) clearTimeout(storageSaveHandle);
  });

  const share = createMemo(() => buildShareUrl(source()));

  // Keep the browser tab title in sync with the UI language.
  createEffect(() => {
    document.title = t('appTitle');
  });

  function handleParse(ps: ParserState) {
    setParsePayload({
      html: ps.html,
      serialized: ps.serialized,
      diagJson: ps.diagJson,
      nodesJson: ps.nodesJson,
      headings: ps.headings,
      profile: ps.profile,
      parseDurationMs: ps.parseDurationMs,
      byteLen: ps.byteLen,
    });
  }

  async function copyShareUrl() {
    const { url, tooLong } = share();
    if (tooLong) {
      flashToast(t('shareTitleTooLong'));
      return;
    }
    try {
      await navigator.clipboard.writeText(url);
      flashToast(t('toastUrlCopied'));
    } catch (err) {
      logError('Clipboard write failed:', err);
      flashToast(t('toastClipboardFail'));
    }
  }

  let toastHandle: number | undefined;
  function flashToast(msg: string) {
    setToast(msg);
    if (toastHandle !== undefined) clearTimeout(toastHandle);
    toastHandle = window.setTimeout(() => setToast(null), 2500);
  }

  return (
    <div class="app">
      <header class="app-header">
        <div class="brand">
          <h1>{t('appTitle')}</h1>
          <p class="tagline">
            <a href="https://github.com/P4suta/aozora" target="_blank" rel="noopener noreferrer">
              aozora
            </a>{' '}
            {t('tagline')}
          </p>
        </div>
        <div class="header-controls">
          <div class="layout-mode-group" role="group" aria-label={t('layoutGroup')}>
            <button
              type="button"
              class={`layout-btn ${layoutMode() === 'editor' ? 'active' : ''}`}
              onClick={() => setLayoutMode('editor')}
              aria-label={t('layoutEditor')}
              title={t('layoutEditorShort')}
            >
              ⌨
            </button>
            <button
              type="button"
              class={`layout-btn ${layoutMode() === 'split' ? 'active' : ''}`}
              onClick={() => setLayoutMode('split')}
              aria-label={t('layoutSplit')}
              title={t('layoutSplitShort')}
            >
              ⇆
            </button>
            <button
              type="button"
              class={`layout-btn ${layoutMode() === 'preview' ? 'active' : ''}`}
              onClick={() => setLayoutMode('preview')}
              aria-label={t('layoutPreview')}
              title={t('layoutPreviewShort')}
            >
              👁
            </button>
          </div>
          <SampleLoader
            onPick={(text, title) => {
              setSource(text);
              flashToast(tf('toastSampleLoaded', { title }));
            }}
          />
          <button
            type="button"
            class="palette-btn"
            onClick={() => setPaletteOpen(true)}
            title={t('paletteOpenTitle')}
            aria-label={t('paletteOpen')}
          >
            <span class="btn-icon">⌘</span>
            <span class="btn-text">{t('paletteText')}</span>
          </button>
          <button
            type="button"
            class="guide-btn"
            onClick={() => setShowGuide(true)}
            title={t('guideOpen')}
            aria-label={t('guideOpen')}
          >
            <span class="btn-icon">📖</span>
            <span class="btn-text">{t('guideText')}</span>
          </button>
          <SettingsPanel
            view={editorView()}
            onResetStorage={() => {
              clearStoredSource();
              setSource(DEFAULT_TEXT);
              flashToast(t('toastStorageReset'));
            }}
          />
          <button
            type="button"
            class="share-btn"
            onClick={copyShareUrl}
            disabled={share().tooLong}
            title={share().tooLong ? t('shareTitleTooLong') : t('shareTitle')}
            aria-label={t('shareLabel')}
          >
            <span class="btn-icon">🔗</span>
            <span class="btn-text">{t('shareLabel')}</span>
          </button>
        </div>
      </header>
      <Show when={wasmError()}>
        <div class="error-banner error-banner-critical" role="alert">
          <div class="error-banner-title">{t('wasmErrorTitle')}</div>
          <div class="error-banner-detail">
            <code>{wasmError()}</code>
          </div>
          <div class="error-banner-hint">
            {t('wasmErrorHint')}
            <button
              type="button"
              class="error-banner-action"
              onClick={() => location.reload()}
            >
              {t('wasmErrorReload')}
            </button>
          </div>
        </div>
      </Show>
      <Show when={!wasmReady() && !wasmError()}>
        <div class="status-banner">{t('wasmLoading')}</div>
      </Show>
      <main class={`app-main mode-${layoutMode()}`}>
        <section class="pane editor-pane">
          <div class="pane-title">{t('editorPaneTitle')}</div>
          <Editor
            value={source()}
            onInput={(v) => setSource(v)}
            onParse={handleParse}
            onReady={(view) => setEditorView(view)}
            onOpenPalette={() => setPaletteOpen(true)}
          />
        </section>
        <section class="pane preview-pane-wrapper">
          <div class="pane-title-row">
            <span class="pane-title">{t('outputPaneTitle')}</span>
            <PerfBadge
              parseDurationMs={parsePayload().parseDurationMs}
              byteLen={parsePayload().byteLen}
              profile={parsePayload().profile}
            />
          </div>
          <PreviewPane
            html={parsePayload().html}
            serialized={parsePayload().serialized}
            diagJson={parsePayload().diagJson}
            nodesJson={parsePayload().nodesJson}
            headings={parsePayload().headings}
            view={editorView()}
          />
        </section>
      </main>
      <NotationGuide open={showGuide()} onClose={() => setShowGuide(false)} />
      <CommandPalette
        open={paletteOpen()}
        view={editorView()}
        onClose={() => setPaletteOpen(false)}
      />
      <Show when={toast()}>
        <div class="toast" role="status" aria-live="polite" aria-atomic="true">
          {toast()}
        </div>
      </Show>
      <footer class="app-footer">
        <a href="https://p4suta.github.io/aozora/" target="_blank" rel="noopener noreferrer">
          Handbook
        </a>
        {' · '}
        <a
          href="https://p4suta.github.io/aozora/api/aozora/"
          target="_blank"
          rel="noopener noreferrer"
        >
          API reference
        </a>
        {' · '}
        <a
          href="https://github.com/P4suta/aozora"
          target="_blank"
          rel="noopener noreferrer"
        >
          GitHub
        </a>
        <Show when={wasmVersion()}>
          {(v) => (
            <>
              {' · '}
              <span class="app-version">aozora {v()}</span>
            </>
          )}
        </Show>
      </footer>
    </div>
  );
}
