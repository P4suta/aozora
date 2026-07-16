import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import type { EditorView } from '@codemirror/view';
import { WRAP_PALETTE, getWrapCommand } from '../editor';
import { fuzzyRank } from '../editor/fuzzy';
import { t, type MessageKey } from '../i18n';

interface CommandPaletteProps {
  open: boolean;
  view: EditorView | null;
  onClose: () => void;
}

// Wrap-command id → catalogue key, so each command's label is translated (the
// `WRAP_SHAPES` descriptions are module-level constants and can't call `t()`).
const CMD_KEY: Record<string, MessageKey> = {
  'aozora.wrap.ruby': 'cmdRuby',
  'aozora.wrap.angleQuote': 'cmdAngleQuote',
  'aozora.wrap.bouten': 'cmdBouten',
  'aozora.wrap.kagikakko': 'cmdKagikakko',
  'aozora.wrap.kikkou': 'cmdKikkou',
  'aozora.wrap.chuki': 'cmdChuki',
};

function describe(id: string): string {
  const key = CMD_KEY[id];
  return key ? t(key) : id;
}

// Surfaces the selection-wrap commands — especially the three full-width-bracket
// shapes (「」/〔〕/［＃］) whose trigger keys aren't typeable (roadmap S12-Q2).
// Overlay discipline: Escape closes, focus moves into the modal on open and
// returns to the previously-focused element on close.
export default function CommandPalette(props: CommandPaletteProps) {
  const [query, setQuery] = createSignal('');
  const [selected, setSelected] = createSignal(0);
  let inputRef!: HTMLInputElement;

  const results = createMemo(() =>
    fuzzyRank(query(), WRAP_PALETTE, (c) => `${c.id} ${describe(c.id)}`),
  );

  createEffect(() => {
    if (!props.open) return;
    setQuery('');
    setSelected(0);
    const previouslyFocused = document.activeElement as HTMLElement | null;
    queueMicrotask(() => inputRef?.focus());
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        props.onClose();
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => {
      window.removeEventListener('keydown', onKey);
      previouslyFocused?.focus?.();
    });
  });

  // Keep the highlighted row in range as the filtered list shrinks.
  createEffect(() => {
    const n = results().length;
    if (selected() >= n) setSelected(Math.max(0, n - 1));
  });

  function run(id: string) {
    const cmd = getWrapCommand(id);
    if (cmd && props.view) {
      cmd(props.view);
      props.view.focus();
    }
    props.onClose();
  }

  function onInputKey(e: KeyboardEvent) {
    const n = results().length;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelected((i) => (n === 0 ? 0 : (i + 1) % n));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelected((i) => (n === 0 ? 0 : (i - 1 + n) % n));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const item = results()[selected()];
      if (item) run(item.id);
    }
  }

  return (
    <Show when={props.open}>
      <div
        class="command-palette-backdrop"
        onClick={(e) => {
          if (e.target === e.currentTarget) props.onClose();
        }}
      >
        <div
          class="command-palette-modal"
          role="dialog"
          aria-modal="true"
          aria-label={t('paletteAriaLabel')}
        >
          <input
            ref={inputRef}
            type="text"
            class="command-palette-input"
            placeholder={t('palettePlaceholder')}
            value={query()}
            onInput={(e) => {
              setQuery(e.currentTarget.value);
              setSelected(0);
            }}
            onKeyDown={onInputKey}
            aria-label={t('paletteSearchLabel')}
          />
          <ul class="command-palette-list" role="listbox">
            <For each={results()}>
              {(cmd, i) => (
                <li
                  class={
                    i() === selected() ? 'command-palette-item active' : 'command-palette-item'
                  }
                  role="option"
                  aria-selected={i() === selected()}
                  onClick={() => run(cmd.id)}
                  onMouseEnter={() => setSelected(i())}
                >
                  <span class="command-palette-desc">{describe(cmd.id)}</span>
                  <span class="command-palette-id">{cmd.id}</span>
                </li>
              )}
            </For>
            <Show when={results().length === 0}>
              <li class="command-palette-empty">{t('paletteEmpty')}</li>
            </Show>
          </ul>
        </div>
      </div>
    </Show>
  );
}
