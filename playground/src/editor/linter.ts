import { linter, lintGutter, type Diagnostic } from '@codemirror/lint';
import type { EditorView } from '@codemirror/view';
import {
  byteToUtf16,
  parserStateField,
  type DiagnosticEntry,
  type ParserState,
} from './parserState';

/**
 * Map raw diagnostic `kind` from aozora-wasm to a CM6 severity +
 * Japanese message. Unknown kinds fall through as `info` with the
 * raw kind name displayed.
 *
 * Wording is loosely modelled on aozora-tools'
 * `crates/aozora-lsp/src/diagnostics.rs` but tuned for casual
 * playground use rather than typesetter precision.
 */
function classify(entry: DiagnosticEntry): { severity: Diagnostic['severity']; message: string } {
  switch (entry.kind) {
    case 'unclosed_bracket':
      return { severity: 'error', message: '括弧が閉じられていません' };
    case 'unmatched_close':
      return { severity: 'error', message: '対応する開き括弧がありません' };
    case 'source_contains_pua': {
      const hex = entry.codepoint
        ? `U+${entry.codepoint.toString(16).toUpperCase().padStart(4, '0')}`
        : '不明';
      return {
        severity: 'warning',
        message: `Private Use Area の文字が含まれています (${hex})`,
      };
    }
    case 'residual_annotation_marker':
      return { severity: 'warning', message: '注記マーカーが残存しています' };
    default:
      return { severity: 'info', message: entry.kind };
  }
}

function lintSource(view: EditorView): readonly Diagnostic[] {
  const ps: ParserState = view.state.field(parserStateField);
  const entries = ps.diagnostics;
  if (entries.length === 0) return [];
  const docLen = view.state.doc.length;
  const out: Diagnostic[] = [];
  for (const entry of entries) {
    const { severity, message } = classify(entry);
    let from = byteToUtf16(ps, entry.span.start);
    let to = byteToUtf16(ps, entry.span.end);
    // Clamp to doc bounds; widen 0-width diagnostics by 1 so they
    // are visible in the gutter / underline.
    if (from < 0) from = 0;
    if (to > docLen) to = docLen;
    if (from === to) {
      if (to < docLen) to = from + 1;
      else if (from > 0) from = to - 1;
    }
    if (from > to) continue;
    out.push({ from, to, severity, message, source: 'aozora' });
  }
  return out;
}

export const aozoraLinter = linter(lintSource, {
  // The parse runs synchronously in parserState. We do not want to
  // throttle squigglies; latency stays sub-millisecond even on the
  // bouten.afm benchmark. Override the default 750ms.
  delay: 50,
});

export const aozoraLintGutter = lintGutter();
