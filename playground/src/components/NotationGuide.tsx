import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { marked, type Tokens } from 'marked';
import notationGuideSource from '../notation-guide.md?raw';
import { t } from '../i18n';

interface NotationGuideProps {
  open: boolean;
  onClose: () => void;
}

interface TocEntry {
  id: string;
  text: string;
  level: number;
}

/**
 * Pre-compute markdown → HTML + 目次（TOC）を 1 回だけ。
 *
 * 目次は h2 / h3 のみを対象。安定 ID は `nh-${index}` の連番にして、
 * 日本語見出しでも slugify 問題を起こさない。
 *
 * inject 側（HTML レンダリング後）と TOC 抽出側で同じ counter を回し
 * て id を整合させる。
 */
const { html: RENDERED_HTML, toc: TOC } = (() => {
  marked.setOptions({ gfm: true, breaks: false });

  const tokens = marked.lexer(notationGuideSource);
  const toc: TocEntry[] = [];
  let counter = 0;
  for (const tok of tokens) {
    if (tok.type !== 'heading') continue;
    const h = tok as Tokens.Heading;
    if (h.depth !== 2 && h.depth !== 3) continue;
    toc.push({ id: `nh-${counter}`, text: h.text, level: h.depth });
    counter++;
  }

  const raw = marked.parse(notationGuideSource, { async: false }) as string;
  let inject = 0;
  const html = raw.replace(/<h([23])(\s[^>]*)?>/g, (_match, lvl, attrs) => {
    const id = `nh-${inject++}`;
    return `<h${lvl} id="${id}"${attrs ?? ''}>`;
  });
  return { html, toc };
})();

export default function NotationGuide(props: NotationGuideProps) {
  const html = createMemo(() => RENDERED_HTML);
  const [activeId, setActiveId] = createSignal<string>(TOC[0]?.id ?? '');
  let bodyRef!: HTMLDivElement;

  let modalRef!: HTMLDivElement;

  // Close on Escape, restore body scroll lock on close, focus-trap.
  createEffect(() => {
    if (!props.open) return;
    const previouslyFocused = document.activeElement as HTMLElement | null;
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';

    // Wait one tick for the modal to be in the DOM before reading focusables.
    queueMicrotask(() => {
      if (!modalRef) return;
      const focusables = collectFocusables(modalRef);
      focusables[0]?.focus();
    });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        props.onClose();
        return;
      }
      if (e.key !== 'Tab' || !modalRef) return;
      const focusables = collectFocusables(modalRef);
      if (focusables.length === 0) return;
      const first = focusables[0]!;
      const last = focusables[focusables.length - 1]!;
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => {
      document.body.style.overflow = prevOverflow;
      window.removeEventListener('keydown', onKey);
      previouslyFocused?.focus?.();
    });
  });

  function collectFocusables(container: HTMLElement): HTMLElement[] {
    return Array.from(
      container.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    ).filter((el) => el.offsetParent !== null);
  }

  function jumpTo(id: string) {
    if (!bodyRef) return;
    const target = bodyRef.querySelector<HTMLElement>(`#${CSS.escape(id)}`);
    if (!target) return;
    const offset = target.offsetTop - bodyRef.offsetTop;
    bodyRef.scrollTo({ top: offset - 8, behavior: 'smooth' });
    setActiveId(id);
  }

  return (
    <Show when={props.open}>
      <div
        class="notation-guide-backdrop"
        onClick={(e) => {
          if (e.target === e.currentTarget) props.onClose();
        }}
      >
        <div
          class="notation-guide-modal"
          role="dialog"
          aria-modal="true"
          aria-label={t('guideModalLabel')}
          ref={modalRef}
        >
          <header class="notation-guide-header">
            <h2>{t('guideModalHeader')}</h2>
            <button
              type="button"
              class="notation-guide-close"
              onClick={props.onClose}
              aria-label={t('close')}
            >
              ×
            </button>
          </header>
          <div class="notation-guide-content">
            <Show when={TOC.length > 0}>
              <nav class="notation-guide-toc" aria-label="目次">
                <ul>
                  <For each={TOC}>
                    {(entry) => (
                      <li class={`toc-l${entry.level}`}>
                        <button
                          type="button"
                          class={
                            activeId() === entry.id ? 'toc-link active' : 'toc-link'
                          }
                          onClick={() => jumpTo(entry.id)}
                        >
                          {entry.text}
                        </button>
                      </li>
                    )}
                  </For>
                </ul>
              </nav>
            </Show>
            <div class="notation-guide-body" ref={bodyRef} innerHTML={html()} />
          </div>
        </div>
      </div>
    </Show>
  );
}
