import { createMemo, For, Show } from 'solid-js';
import type { DiagnosticEntry, WireEnvelope } from '../types';

interface DiagnosticsViewProps {
  json: string;
}

export default function DiagnosticsView(props: DiagnosticsViewProps) {
  const data = createMemo<DiagnosticEntry[]>(() => {
    try {
      const env: WireEnvelope<DiagnosticEntry> = JSON.parse(props.json);
      return env.data ?? [];
    } catch {
      return [];
    }
  });

  return (
    <Show
      when={data().length > 0}
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
          <For each={data()}>
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
