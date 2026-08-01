import type {
  LocalizedText,
  PlaygroundAdapter,
  PlaygroundGuide,
  PlaygroundSample,
} from '@aozora/playground-ui';

import wasmPackage from '../../crates/aozora-wasm/pkg/package.json';
import type { EngineAwareEditorController } from './editor-controller';
import { setEditorLocale } from './i18n';
import { createRetryableLoader } from './retryableLoader';
import { DEFAULT_SAMPLE_ID, SAMPLES } from './samples';

type EngineModule = typeof import('./adapter-engine');
type EditorModule = typeof import('./editor-controller');

let engineReady = false;
const activeEditors = new Set<EngineAwareEditorController>();

const loadEngine = createRetryableLoader<EngineModule>(
  () => import('./adapter-engine'),
);
const loadEditor = createRetryableLoader<EditorModule>(
  () => import('./editor-controller'),
);

const sampleEnglishTitles: Readonly<Record<string, string>> = {
  ruby: 'Explicit ruby',
  'ruby-implicit': 'Implicit ruby',
  bouten: 'Emphasis dots',
  'ruby-bouten': 'Ruby and emphasis dots',
  indent: 'Indented block',
  gaiji: 'JIS X 0213 gaiji',
  'angle-quote': 'Double angle brackets',
  'page-break': 'Page break',
  heading: 'Large heading',
  tcy: 'Tate-chu-yoko',
  warichu: 'Inline note',
  keigakomi: 'Framed block',
  bousen: 'Underline',
  jitsuki: 'End alignment',
  'kitchen-sink': 'Mixed notation',
  'long-form': 'Long-form performance sample',
};

const samples: readonly PlaygroundSample[] = [
  ...SAMPLES.filter((sample) => sample.id === DEFAULT_SAMPLE_ID),
  ...SAMPLES.filter((sample) => sample.id !== DEFAULT_SAMPLE_ID),
].map((sample) => ({
  id: sample.id,
  title: {
    ja: `${sample.title} — ${sample.source}`,
    en: `${sampleEnglishTitles[sample.id] ?? sample.title} — ${sample.source}`,
  },
  source: sample.text,
}));

const guide: PlaygroundGuide = {
  title: {
    ja: '青空文庫記法ガイド',
    en: 'Aozora notation guide',
  },
  introduction: {
    ja: 'ルビ、傍点、外字、字下げなど、青空文庫の組版記法を入力できます。',
    en: 'Write Aozora Bunko typography including ruby, emphasis dots, gaiji, and indentation.',
  },
  sections: [
    {
      id: 'ruby',
      title: { ja: 'ルビ', en: 'Ruby' },
      body: {
        ja: '漢字の直後に読みを置くか、縦線で対象範囲を明示します。',
        en: 'Place a reading after kanji, or use a vertical bar to mark an explicit base.',
      },
      example: '悟浄《ごじょう》と｜雑司ヶ谷《ぞうしがや》',
    },
    {
      id: 'annotations',
      title: { ja: '注記', en: 'Annotations' },
      body: {
        ja: '［＃…］で傍点、字下げ、見出し、縦中横などの組版を指定します。',
        en: 'Use ［＃…］ annotations for emphasis, indentation, headings, tate-chu-yoko, and more.',
      },
      example: '明治33［＃「33」は縦中横］年',
    },
    {
      id: 'commands',
      title: { ja: '編集支援', en: 'Editing assistance' },
      body: {
        ja: '文字列を選択してコマンドを実行すると記法で囲めます。Ctrl/⌘+Shift+Pで検索できます。',
        en: 'Select text and run a command to wrap it in notation. Search commands with Ctrl/⌘+Shift+P.',
      },
    },
    {
      id: 'official',
      title: { ja: '公式注記一覧', en: 'Official annotation list' },
      body: {
        ja: '全記法の詳細は青空文庫公式の注記一覧を参照してください。',
        en: 'See Aozora Bunko’s official annotation list for the complete notation reference.',
      },
      href: 'https://www.aozora.gr.jp/annotation/',
    },
  ],
};

const commandLabels: Readonly<Record<string, LocalizedText>> = {
  'aozora.wrap.ruby': { ja: 'ルビ', en: 'Ruby' },
  'aozora.wrap.angleQuote': { ja: '二重山括弧', en: 'Double angle brackets' },
  'aozora.wrap.bouten': { ja: '傍点', en: 'Emphasis dots' },
  'aozora.wrap.kagikakko': { ja: '鉤括弧で囲む', en: 'Wrap in 「」' },
  'aozora.wrap.kikkou': { ja: '亀甲括弧で囲む', en: 'Wrap in 〔〕' },
  'aozora.wrap.chuki': { ja: '注記で囲む', en: 'Wrap in ［＃］' },
};

const commandShortcuts: Readonly<Record<string, string>> = {
  'aozora.wrap.ruby': 'Ctrl/⌘+Alt+R',
  'aozora.wrap.angleQuote': 'Ctrl/⌘+Alt+Shift+R',
  'aozora.wrap.bouten': 'Ctrl/⌘+Alt+B',
};

export const aozoraPlaygroundAdapter: PlaygroundAdapter = {
  product: {
    id: 'aozora',
    name: 'Aozora Notation',
    shortName: 'aozora',
    description: {
      ja: '青空文庫記法の執筆・検証環境',
      en: 'Authoring and validation for Aozora Bunko notation',
    },
    repositoryUrl: 'https://github.com/P4suta/aozora',
    engineVersion: wasmPackage.version,
  },
  samples,
  guide,
  commands: Object.entries(commandLabels).map(([id, label]) => ({
    id,
    label,
    ...(commandShortcuts[id] ? { shortcut: commandShortcuts[id] } : {}),
  })),
  settings: [
    {
      id: 'halfWidthConversion',
      label: {
        ja: '半角→全角の即時変換',
        en: 'Instant half-width → full-width',
      },
      description: {
        ja: '[ → ［、| → ｜などを入力時に変換します。',
        en: 'Convert characters such as [ → ［ and | → ｜ while typing.',
      },
      defaultValue: true,
    },
    {
      id: 'gaijiInlayHints',
      label: { ja: '外字インレイヒント', en: 'Gaiji inlay hints' },
      description: {
        ja: '外字注記の後ろに解決された文字を表示します。',
        en: 'Show the resolved character after a gaiji annotation.',
      },
      defaultValue: true,
    },
  ],
  createEditorDuringInitialization: true,
  setLocale(locale) {
    setEditorLocale(locale);
    for (const editor of activeEditors) editor.refreshLocale();
  },
  async initialize() {
    const engine = await loadEngine();
    await engine.initializeEngine();
    engineReady = true;
    for (const editor of activeEditors) editor.enableEngineFeatures();
  },
  async analyze(source, context) {
    const engine = await loadEngine();
    return engine.analyze(source, context);
  },
  async createEditor(parent, initialValue, onChange) {
    const { createEditor } = await loadEditor();
    const editor = createEditor(parent, initialValue, onChange, engineReady);
    activeEditors.add(editor);
    return {
      ...editor,
      destroy() {
        activeEditors.delete(editor);
        editor.destroy();
      },
    };
  },
  createPreview(parent) {
    const root = document.createElement('div');
    root.className = 'aozora-notation';
    parent.replaceChildren(root);
    let previousHtml: string | null = null;
    return {
      update(html, direction) {
        root.classList.toggle('aozora-vertical', direction === 'vertical');
        if (html !== previousHtml) {
          root.innerHTML = html;
          previousHtml = html;
        }
      },
      destroy() {
        root.remove();
      },
    };
  },
};
