import { type Diagnostic, linter, lintGutter } from '@codemirror/lint';
import type { EditorView } from '@codemirror/view';
import { t, tf } from '../i18n';
import {
  byteToUtf16,
  type DiagnosticEntry,
  type ParserState,
  parserStateField,
} from './parserState';

function classify(entry: DiagnosticEntry): {
  severity: Diagnostic['severity'];
  message: string;
} {
  const severity = diagnosticSeverity(entry.severity);
  switch (entry.kind) {
    case 'unclosed_bracket':
      return { severity, message: t('lintUnclosed') };
    case 'unmatched_close':
      return { severity, message: t('lintUnmatched') };
    case 'source_contains_pua': {
      const hex = entry.codepoint
        ? `U+${entry.codepoint.toString(16).toUpperCase().padStart(4, '0')}`
        : 'U+????';
      return {
        severity,
        message: tf('lintPua', { hex }),
      };
    }
    case 'residual_annotation_marker':
      return { severity, message: t('lintStrayMarker') };
    default:
      return { severity, message: entry.kind };
  }
}

export function diagnosticSeverity(
  severity: DiagnosticEntry['severity'],
): Diagnostic['severity'] {
  return severity === 'note' ? 'info' : severity;
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
