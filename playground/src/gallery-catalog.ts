import type { Locale } from '@aozora/playground-ui';

const ja = {
  back: 'Playgroundへ戻る',
  description:
    '主要な記法を、実レンダラ出力で横書き・縦書きの両方に表示します。',
  failure: 'WebAssemblyの初期化に失敗しました',
  horizontal: '横書き',
  loading: 'レンダラーを初期化中…',
  loadingLabel: 'レンダラーを初期化中',
  retry: '再試行',
  retryHint: 'もう一度読み込みを試してください。',
  title: '青空文庫記法ギャラリー',
  vertical: '縦書き',
} as const;

type GalleryMessageKey = keyof typeof ja;

const en: Record<GalleryMessageKey, string> = {
  back: 'Back to the playground',
  description:
    'Representative notation rendered by the real engine in horizontal and vertical writing.',
  failure: 'WebAssembly failed to initialize',
  horizontal: 'Horizontal',
  loading: 'Initializing renderer…',
  loadingLabel: 'Initializing renderer',
  retry: 'Retry',
  retryHint: 'Try loading the renderer again.',
  title: 'Aozora notation gallery',
  vertical: 'Vertical',
};

export const GALLERY_CATALOG: Record<
  Locale,
  Record<GalleryMessageKey, string>
> = { ja, en };
