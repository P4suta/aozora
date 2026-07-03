/* @refresh reload */
import { render } from 'solid-js/web';
import { For } from 'solid-js';
import { bootstrapTheme } from './theme';
import { bootstrapLang } from './i18n';
import { ensureWasmReady, Document } from './wasm-loader';
import './styles.css';
// レンダラ所有の正準記法スタイルシート（単一の権威）。main.tsx と同じ二枚を
// 同順で読み込み、ギャラリーも実 render 出力を実 CSS で表示する。テーマ橋渡しと
// 枠のレイアウトは続く aozora.css が上書きする。
import '../../crates/aozora-render/assets/aozora-notation.css';
import './aozora.css';
import './gallery.css';

// 各「見えて装飾される」記法ファミリ（hidden な aozora-directive は除く）を
// 一つずつ demonstrate する fixture。文字列は samples.ts と同じく 青空文庫の
// パブリックドメイン作品由来の実抜粋で、いずれも診断ゼロで render される。
interface Fixture {
  /** `<section data-family>` のフック。E2E セレクタの安定キー。 */
  family: string;
  /** 日本語ラベル（samples.ts の title と同様、i18n カタログには載せない）。 */
  label: string;
  /** 青空文庫記法のソース。`new Document(text).toHtml()` に渡す。 */
  text: string;
}

const FIXTURES: Fixture[] = [
  {
    family: 'ruby',
    label: 'ルビ',
    text: '悟浄《ごじょう》の肉体はもはや疲れ切っていた。',
  },
  {
    family: 'bouten',
    label: '傍点',
    text: 'ふらんす［＃「ふらんす」に傍点］はあまりに遠し',
  },
  {
    family: 'tcy',
    label: '縦中横',
    text: '（［＃縦中横］10［＃縦中横終わり］）「かいともし、とうよ」',
  },
  {
    family: 'kaeriten',
    label: '返り点',
    text: '漢文［＃上二］また［＃下二］。',
  },
  {
    family: 'gaiji',
    label: '外字',
    text: '美女、瞳を※［＃「目＋爭」、第3水準1-88-85］《みは》る。',
  },
  {
    family: 'angle-quote',
    label: '二重山括弧',
    text: '≪風は冷気をつつんでゐる≫',
  },
  {
    family: 'warichu',
    label: '割り注',
    text: '一、乳油［＃割り注］洋名バタ［＃割り注終わり］',
  },
];

interface Panel {
  family: string;
  label: string;
  html: string;
}

/**
 * Parse one fixture to HTML through the real WASM engine. Mirrors the
 * parserState.ts call pattern: construct a `Document`, serialize to HTML, then
 * `free()` the wasm-owned handle so the parser arena is released immediately.
 */
function renderFixture(text: string): string {
  const doc = new Document(text);
  const html = doc.toHtml();
  doc.free();
  return html;
}

/**
 * The gallery page. Each family renders one `<section data-family>` with a
 * Japanese `<h2>` label and two side-by-side previews of the identical renderer
 * HTML: horizontal (base `.aozora-notation`) and vertical (`.aozora-vertical`).
 * Both mount via `innerHTML` — the same escaped, script-free output
 * `HtmlPreview.tsx` uses — so the canonical sheet is exercised in both writing
 * modes.
 */
function Gallery(props: { panels: Panel[] }) {
  return (
    <main class="gallery">
      <h1 class="gallery-title">青空文庫記法ギャラリー</h1>
      <p class="gallery-lead">
        主要な記法ファミリを、実レンダラ出力で横書き・縦書きの両方に表示します。
      </p>
      <For each={props.panels}>
        {(panel) => (
          <section class="gallery-section" data-family={panel.family}>
            <h2 class="gallery-family-label">{panel.label}</h2>
            <div class="gallery-columns">
              <div class="gallery-h">
                <span class="gallery-mode">横書き</span>
                <div class="html-preview aozora-notation" innerHTML={panel.html} />
              </div>
              <div class="gallery-v">
                <span class="gallery-mode">縦書き</span>
                <div
                  class="html-preview aozora-notation aozora-vertical"
                  innerHTML={panel.html}
                />
              </div>
            </div>
          </section>
        )}
      </For>
    </main>
  );
}

bootstrapTheme();
bootstrapLang();

const root = document.getElementById('root');
if (!root) {
  throw new Error('Missing #root element');
}

// The gallery is static — no per-keystroke reparse — so every fixture is parsed
// once, up front, after the parser is ready, and then the page mounts. Awaiting
// ensureWasmReady() before constructing any `Document` mirrors how App.tsx gates
// the editor on wasmReady; the first rendered element (the ルビ section) doubles
// as the E2E's "engine ready" signal.
void ensureWasmReady().then(() => {
  const panels: Panel[] = FIXTURES.map((f) => ({
    family: f.family,
    label: f.label,
    html: renderFixture(f.text),
  }));
  render(() => <Gallery panels={panels} />, root);
});
