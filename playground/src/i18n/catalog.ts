import type { Locale } from './types';

// UI string catalogue (#336 D-7). `ja` is the source of truth and defines the
// key set; `MessageKey` is `keyof typeof ja`, and `en` is typed
// `Record<MessageKey, string>` so a missing OR extra English key fails `tsc`.
// `{x}` placeholders are filled by `tf()`. Literary sample text (samples.ts)
// and the notation-guide body are out of scope — they stay Japanese.

const ja = {
  // App — header / panes / banners / toasts
  appTitle: '青空文庫記法 Playground',
  tagline: '— Rust + WebAssembly 製の高速パーサー。入力に応じてリアルタイムに HTML を生成します。',
  layoutGroup: 'レイアウト切替',
  layoutEditor: 'エディタのみ表示',
  layoutEditorShort: 'エディタのみ',
  layoutSplit: '分割表示',
  layoutSplitShort: '分割',
  layoutPreview: 'プレビューのみ表示',
  layoutPreviewShort: 'プレビューのみ',
  paletteOpen: 'コマンドパレットを開く',
  paletteOpenTitle: 'コマンドパレットを開く（Ctrl/⌘+Shift+P）',
  paletteText: 'コマンド',
  guideOpen: '記法ガイドを開く',
  guideText: '記法ガイド',
  guideModalLabel: '青空文庫記法 完全リファレンス',
  guideModalHeader: '📖 青空文庫記法 リファレンス',
  close: '閉じる',
  // HtmlPreview writing-mode toggle
  writingToHorizontal: '横書きに切り替え',
  writingToVertical: '縦書きに切り替え',
  writingHorizontalLabel: '↻ 横書',
  writingVerticalLabel: '↺ 縦書',
  // hover
  hoverUnresolved: '(未解決)',
  shareLabel: '共有 URL をコピー',
  shareTitle: 'URL をコピー',
  shareTitleTooLong: 'テキストが長すぎて URL 共有できません',
  wasmErrorTitle: '⚠ WASM の初期化に失敗しました',
  wasmErrorHint:
    'WebAssembly が無効化されている、もしくは CSP / 拡張機能によりロードがブロックされている可能性があります。',
  wasmErrorReload: 'ページを再読み込み',
  wasmLoading: 'WASM 初期化中…',
  editorPaneTitle: '入力（青空文庫記法）',
  outputPaneTitle: '出力',
  editorPlaceholder: '青空文庫記法を入力…\nサンプル選択またはガイドから読み始めてもOK',
  editorDisabled:
    'WASM が読み込めなかったため、エディタを起動できません。<br/>上部のバナーから再読み込みしてください。',
  sampleLabel: 'サンプル:',
  sampleSelectPlaceholder: '選択してください…',
  toastSampleLoaded: 'サンプル「{title}」を読み込みました',
  toastStorageReset: '保存をリセットしました',
  toastUrlCopied: 'URL をコピーしました',
  toastClipboardFail: 'クリップボードに書き込めませんでした',
  toastShareDecodeFail: '共有 URL のデコードに失敗しました',
  toastSaveFail: '編集内容の保存に失敗しました（容量不足の可能性）',

  // SettingsPanel
  settingsTrigger: 'エディタ設定',
  settingHalfWidth: '半角→全角の即時変換',
  settingHalfWidthSub: '[ → ［ 、 | → ｜ など 8 種',
  settingInlay: '外字インレイヒント',
  settingInlaySub: '※［＃...］の後ろに →解決字 を表示',
  settingTheme: 'テーマ',
  settingThemeSub: 'Auto は OS 設定に追従',
  settingReset: '保存をリセット',
  settingResetSub: 'localStorage の編集内容を消去（共有 URL は影響なし）',
  settingLanguage: '言語',
  settingLanguageSub: 'UI の表示言語',

  // PerfBadge
  perfExpand: 'クリックでメソッド別レイテンシを展開',
  perfProfile: '性能プロファイル',
  perfHeader: 'メソッド別レイテンシ',
  perfColMethod: 'メソッド',
  perfColTime: '時間',
  perfColThroughput: 'スループット',
  perfTotal: '合計',
  perfFooterPre: '計測は',
  perfFooterMid: '経由。 PerfBadge の数値は',
  perfFooterPost: 'のみ。',

  // CommandPalette
  palettePlaceholder: 'コマンドを検索…（囲み記法など）',
  paletteEmpty: '一致するコマンドがありません',
  paletteAriaLabel: 'コマンドパレット',
  paletteSearchLabel: 'コマンド検索',

  // Wrap-command descriptions (palette + completion)
  cmdRuby: 'ルビ',
  cmdAngleQuote: '二重山括弧',
  cmdBouten: '傍点',
  cmdKagikakko: '鉤括弧で囲む',
  cmdKikkou: '亀甲括弧で囲む',
  cmdChuki: '注記で囲む',

  // Editor completion labels / details
  compAnn: '＃ アノテーション',
  compAnnDetail: '［＃...］ 一行注記の即時テンプレ',
  compRuby: '｜ ルビ（明示）',
  compRubyDetail: '｜base《reading》 で明示ルビ',
  compImplicitRuby: '《 ルビ（暗黙）',
  compImplicitRubyDetail: '直前の漢字に読みを振る',
  compGaiji: '※ 外字',
  compGaijiDetail: '※［＃「desc」、mencode］',

  // Linter messages
  lintUnclosed: '括弧が閉じられていません',
  lintUnmatched: '対応する開き括弧がありません',
  lintPua: 'Private Use Area の文字が含まれています ({hex})',
  lintStrayMarker: '注記マーカーが残存しています',
};

export type MessageKey = keyof typeof ja;

const en: Record<MessageKey, string> = {
  appTitle: 'Aozora Notation Playground',
  tagline: '— a fast Rust + WebAssembly parser. Renders HTML in real time as you type.',
  layoutGroup: 'Switch layout',
  layoutEditor: 'Editor only',
  layoutEditorShort: 'Editor',
  layoutSplit: 'Split view',
  layoutSplitShort: 'Split',
  layoutPreview: 'Preview only',
  layoutPreviewShort: 'Preview',
  paletteOpen: 'Open command palette',
  paletteOpenTitle: 'Open command palette (Ctrl/⌘+Shift+P)',
  paletteText: 'Commands',
  guideOpen: 'Open notation guide',
  guideText: 'Guide',
  guideModalLabel: 'Aozora notation reference',
  guideModalHeader: '📖 Aozora Notation Reference',
  close: 'Close',
  writingToHorizontal: 'Switch to horizontal writing',
  writingToVertical: 'Switch to vertical writing',
  writingHorizontalLabel: '↻ Horizontal',
  writingVerticalLabel: '↺ Vertical',
  hoverUnresolved: '(unresolved)',
  shareLabel: 'Copy share URL',
  shareTitle: 'Copy URL',
  shareTitleTooLong: 'Text too long to share via URL',
  wasmErrorTitle: '⚠ Failed to initialize WASM',
  wasmErrorHint:
    'WebAssembly may be disabled, or a CSP / browser extension may be blocking the load.',
  wasmErrorReload: 'Reload the page',
  wasmLoading: 'Initializing WASM…',
  editorPaneTitle: 'Input (Aozora notation)',
  outputPaneTitle: 'Output',
  editorPlaceholder: 'Type Aozora notation…\nor start from a sample or the guide',
  editorDisabled:
    'The editor could not start because WASM failed to load.<br/>Use the banner above to reload.',
  sampleLabel: 'Sample:',
  sampleSelectPlaceholder: 'Choose a sample…',
  toastSampleLoaded: 'Loaded the sample “{title}”',
  toastStorageReset: 'Storage has been reset',
  toastUrlCopied: 'URL copied',
  toastClipboardFail: 'Could not write to the clipboard',
  toastShareDecodeFail: 'Failed to decode the share URL',
  toastSaveFail: 'Failed to save your edits (storage may be full)',

  settingsTrigger: 'Editor settings',
  settingHalfWidth: 'Instant half-width → full-width',
  settingHalfWidthSub: '[ → ［, | → ｜, and 6 more',
  settingInlay: 'Gaiji inlay hints',
  settingInlaySub: 'Show the →resolved glyph after ※［＃...］',
  settingTheme: 'Theme',
  settingThemeSub: 'Auto follows the OS setting',
  settingReset: 'Reset storage',
  settingResetSub: 'Clear the localStorage draft (share URLs are unaffected)',
  settingLanguage: 'Language',
  settingLanguageSub: 'UI display language',

  perfExpand: 'Click to expand per-method latency',
  perfProfile: 'Performance profile',
  perfHeader: 'Per-method latency',
  perfColMethod: 'Method',
  perfColTime: 'Time',
  perfColThroughput: 'Throughput',
  perfTotal: 'Total',
  perfFooterPre: 'Measured via',
  perfFooterMid: '. The PerfBadge figures cover only',
  perfFooterPost: '.',

  palettePlaceholder: 'Search commands… (enclosures, etc.)',
  paletteEmpty: 'No matching commands',
  paletteAriaLabel: 'Command palette',
  paletteSearchLabel: 'Command search',

  cmdRuby: 'Ruby',
  cmdAngleQuote: 'Double angle brackets',
  cmdBouten: 'Emphasis dots',
  cmdKagikakko: 'Wrap in 「」',
  cmdKikkou: 'Wrap in 〔〕',
  cmdChuki: 'Wrap in ［＃］',

  compAnn: '＃ Annotation',
  compAnnDetail: '［＃...］ one-line note template',
  compRuby: '｜ Ruby (explicit)',
  compRubyDetail: 'Explicit ruby via ｜base《reading》',
  compImplicitRuby: '《 Ruby (implicit)',
  compImplicitRubyDetail: 'Reading for the preceding kanji',
  compGaiji: '※ Gaiji',
  compGaijiDetail: '※［＃「desc」、mencode］',

  lintUnclosed: 'Unclosed bracket',
  lintUnmatched: 'No matching open bracket',
  lintPua: 'Contains a Private Use Area character ({hex})',
  lintStrayMarker: 'Stray annotation marker',
};

export const CATALOG: Record<Locale, Record<MessageKey, string>> = { ja, en };
