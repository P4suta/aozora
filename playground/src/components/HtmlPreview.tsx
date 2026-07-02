import { createSignal } from 'solid-js';
import { t } from '../i18n';

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
          title={vertical() ? t('writingToHorizontal') : t('writingToVertical')}
        >
          {vertical() ? t('writingHorizontalLabel') : t('writingVerticalLabel')}
        </button>
      </div>
      <div
        class={`html-preview aozora-notation ${vertical() ? 'is-vertical aozora-vertical' : ''}`}
        // aozora-render の出力は <script>/<a href>/on*= を一切 emit せず、テキストは
        // escape_text で <>&"' をエンティティ化済み。innerHTML で挿入しても XSS 経路は
        // 存在しない（crates/aozora-render/src/render_node.rs 確認済み）。
        innerHTML={props.html}
      />
    </div>
  );
}
