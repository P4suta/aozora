import { For, Show } from 'solid-js';
import { EditorView } from '@codemirror/view';
import type { HeadingEntry } from '../editor';

interface OutlinePanelProps {
  /** Pre-computed heading entries from `parserState.headings`. */
  headings: HeadingEntry[];
  view: EditorView | null;
}

export default function OutlinePanel(props: OutlinePanelProps) {
  function jumpTo(entry: HeadingEntry) {
    const view = props.view;
    if (!view) return;
    view.focus();
    view.dispatch({
      effects: EditorView.scrollIntoView(entry.from, { y: 'center' }),
      selection: { anchor: entry.from },
    });
  }

  return (
    <Show
      when={props.headings.length > 0}
      fallback={
        <div class="outline-empty">
          見出し（［＃...］の見出しヒント）が検出されると、ここに一覧が表示されます。
        </div>
      }
    >
      <ul class="outline-list">
        <For each={props.headings}>
          {(entry) => (
            <li class={`outline-item outline-l${entry.level}`}>
              <button type="button" class="outline-link" onClick={() => jumpTo(entry)}>
                <span class="outline-text">{entry.text}</span>
              </button>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}
