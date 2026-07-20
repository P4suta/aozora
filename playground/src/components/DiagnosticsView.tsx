import { For, Show } from 'solid-js';
import type { DiagnosticEntry } from '../editor';

interface DiagnosticsViewProps {
  diagnostics: DiagnosticEntry[];
}

export default function DiagnosticsView(props: DiagnosticsViewProps) {
  return (
    <Show
      when={props.diagnostics.length > 0}
      fallback={
        <div class="diag-empty">
          診断は空です（入力が clean、もしくは入力なし）
        </div>
      }
    >
      <table class="diag-table">
        <thead>
          <tr>
            <th>kind</th>
            <th>span</th>
            <th>extra</th>
          </tr>
        </thead>
        <tbody>
          <For each={props.diagnostics}>
            {(entry) => (
              <tr>
                <td>
                  <code>{entry.kind}</code>
                </td>
                <td>
                  <code>
                    [{entry.span.start}, {entry.span.end})
                  </code>
                </td>
                <td>
                  <Show when={entry.codepoint !== undefined}>
                    <code>U+{entry.codepoint!.toString(16).toUpperCase().padStart(4, '0')}</code>
                  </Show>
                </td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </Show>
  );
}
