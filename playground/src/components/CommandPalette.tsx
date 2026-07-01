import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import type { EditorView } from '@codemirror/view';
import { WRAP_PALETTE, getWrapCommand } from '../editor';
import { fuzzyRank } from '../editor/fuzzy';

interface CommandPaletteProps {
  open: boolean;
  view: EditorView | null;
  onClose: () => void;
}

// Surfaces the selection-wrap commands — especially the three full-width-bracket
// shapes (「」/〔〕/［＃］) whose trigger keys aren't typeable (roadmap S12-Q2).
// Mirrors NotationGuide.tsx's overlay/focus discipline.
export default function CommandPalette(props: CommandPaletteProps) {
  const [query, setQuery] = createSignal('');
  const [selected, setSelected] = createSignal(0);
  let inputRef!: HTMLInputElement;

  const results = createMemo(() =>
    fuzzyRank(query(), WRAP_PALETTE, (c) => `${c.id} ${c.description}`),
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
          aria-label="コマンドパレット"
        >
          <input
            ref={inputRef}
            type="text"
            class="command-palette-input"
            placeholder="コマンドを検索…（囲み記法など）"
            value={query()}
            onInput={(e) => {
              setQuery(e.currentTarget.value);
              setSelected(0);
            }}
            onKeyDown={onInputKey}
            aria-label="コマンド検索"
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
                  <span class="command-palette-desc">{cmd.description}</span>
                  <span class="command-palette-id">{cmd.id}</span>
                </li>
              )}
            </For>
            <Show when={results().length === 0}>
              <li class="command-palette-empty">一致するコマンドがありません</li>
            </Show>
          </ul>
        </div>
      </div>
    </Show>
  );
}
