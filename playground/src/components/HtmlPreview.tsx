import { createSignal } from 'solid-js';

interface HtmlPreviewProps {
  html: string;
}

export default function HtmlPreview(props: HtmlPreviewProps) {
  const [vertical, setVertical] = createSignal(false);
  return (
    <div class="html-preview-shell">
      <div class="html-preview-toolbar">
        <button
          type="button"
          class={`writing-mode-btn ${vertical() ? 'active' : ''}`}
          onClick={() => setVertical((v) => !v)}
          title={vertical() ? '横書きに切り替え' : '縦書きに切り替え'}
        >
          {vertical() ? '↻ 横書' : '↺ 縦書'}
        </button>
      </div>
      <div
        class={`html-preview aozora-doc ${vertical() ? 'is-vertical' : ''}`}
        // aozora-render の出力は <script>/<a href>/on*= を一切 emit せず、テキストは
        // escape_text で <>&"' をエンティティ化済み。innerHTML で挿入しても XSS 経路は
        // 存在しない（crates/aozora-render/src/render_node.rs 確認済み）。
        innerHTML={props.html}
      />
    </div>
  );
}
