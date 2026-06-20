import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import { EditorView } from '@codemirror/view';
import Editor from './components/Editor';
import PreviewPane from './components/PreviewPane';
import SampleLoader from './components/SampleLoader';
import PerfBadge from './components/PerfBadge';
import { ensureWasmReady } from './wasm-loader';
import type { HeadingEntry, ParserState, ProfilePhaseEntry } from './editor';
import { buildShareUrl, readShareTextFromUrl, syncTextToUrl } from './share';
import { clearStoredSource, loadStoredSource, saveSource } from './storage';
import { SAMPLES, DEFAULT_SAMPLE_ID } from './samples';
import NotationGuide from './components/NotationGuide';
import SettingsPanel from './components/SettingsPanel';
import { error as logError } from './logger';

const EMPTY_ENVELOPE = '{"schemaVersion":1,"data":[]}';

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
  const [toast, setToast] = createSignal<string | null>(null);
  const [parsePayload, setParsePayload] = createSignal<ParsePayload>(EMPTY_PAYLOAD);
  const [editorView, setEditorView] = createSignal<EditorView | null>(null);
  const [showGuide, setShowGuide] = createSignal(false);
  // Layout mode for the editor / preview panes. Useful on phones
  // (one-pane focus) but also on desktop when the user wants a
  // wider editor or full-screen preview.
  const [layoutMode, setLayoutMode] = createSignal<'split' | 'editor' | 'preview'>('split');

  onMount(() => {
    ensureWasmReady()
      .then(() => setWasmReady(true))
      .catch((err: unknown) => {
        logError('WASM init failed:', err);
        setWasmError(err instanceof Error ? err.message : String(err));
      });
    if (urlRead.status === 'invalid') {
      // 起動時の URL デコード失敗を通知。setSource は走らないので default に落ちている。
      flashToast('共有 URL のデコードに失敗しました');
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
          flashToast('編集内容の保存に失敗しました（容量不足の可能性）');
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
      flashToast('テキストが長すぎて URL 共有できません');
      return;
    }
    try {
      await navigator.clipboard.writeText(url);
      flashToast('URL をコピーしました');
    } catch (err) {
      logError('Clipboard write failed:', err);
      flashToast('クリップボードに書き込めませんでした');
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
          <h1>青空文庫記法 Playground</h1>
          <p class="tagline">
            <a href="https://github.com/P4suta/aozora" target="_blank" rel="noopener noreferrer">
              aozora
            </a>{' '}
            — Rust + WebAssembly 製の高速パーサー。入力に応じてリアルタイムに HTML を生成します。
          </p>
        </div>
        <div class="header-controls">
          <div class="layout-mode-group" role="group" aria-label="レイアウト切替">
            <button
              type="button"
              class={`layout-btn ${layoutMode() === 'editor' ? 'active' : ''}`}
              onClick={() => setLayoutMode('editor')}
              aria-label="エディタのみ表示"
              title="エディタのみ"
            >
              ⌨
            </button>
            <button
              type="button"
              class={`layout-btn ${layoutMode() === 'split' ? 'active' : ''}`}
              onClick={() => setLayoutMode('split')}
              aria-label="分割表示"
              title="分割"
            >
              ⇆
            </button>
            <button
              type="button"
              class={`layout-btn ${layoutMode() === 'preview' ? 'active' : ''}`}
              onClick={() => setLayoutMode('preview')}
              aria-label="プレビューのみ表示"
              title="プレビューのみ"
            >
              👁
            </button>
          </div>
          <SampleLoader
            onPick={(text, title) => {
              setSource(text);
              flashToast(`サンプル「${title}」を読み込みました`);
            }}
          />
          <button
            type="button"
            class="guide-btn"
            onClick={() => setShowGuide(true)}
            title="記法ガイドを開く"
            aria-label="記法ガイドを開く"
          >
            <span class="btn-icon">📖</span>
            <span class="btn-text">記法ガイド</span>
          </button>
          <SettingsPanel
            view={editorView()}
            onResetStorage={() => {
              clearStoredSource();
              setSource(DEFAULT_TEXT);
              flashToast('保存をリセットしました');
            }}
          />
          <button
            type="button"
            class="share-btn"
            onClick={copyShareUrl}
            disabled={share().tooLong}
            title={share().tooLong ? 'テキストが長すぎて URL 共有できません' : 'URL をコピー'}
            aria-label="共有 URL をコピー"
          >
            <span class="btn-icon">🔗</span>
            <span class="btn-text">共有 URL をコピー</span>
          </button>
        </div>
      </header>
      <Show when={wasmError()}>
        <div class="error-banner error-banner-critical" role="alert">
          <div class="error-banner-title">⚠ WASM の初期化に失敗しました</div>
          <div class="error-banner-detail">
            <code>{wasmError()}</code>
          </div>
          <div class="error-banner-hint">
            WebAssembly が無効化されている、もしくは CSP / 拡張機能によりロードが
            ブロックされている可能性があります。
            <button
              type="button"
              class="error-banner-action"
              onClick={() => location.reload()}
            >
              ページを再読み込み
            </button>
          </div>
        </div>
      </Show>
      <Show when={!wasmReady() && !wasmError()}>
        <div class="status-banner">WASM 初期化中…</div>
      </Show>
      <main class={`app-main mode-${layoutMode()}`}>
        <section class="pane editor-pane">
          <div class="pane-title">入力（青空文庫記法）</div>
          <Editor
            value={source()}
            onInput={(v) => setSource(v)}
            onParse={handleParse}
            onReady={(view) => setEditorView(view)}
          />
        </section>
        <section class="pane preview-pane-wrapper">
          <div class="pane-title-row">
            <span class="pane-title">出力</span>
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
      </footer>
    </div>
  );
}
