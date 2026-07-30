import type {
  AnalyzeContext,
  LocalizedText,
  PlaygroundAnalysis,
  PlaygroundDiagnostic,
  PlaygroundOutlineEntry,
  TextRange,
} from '@aozora/playground-ui';
import type { ContainerPair, Diagnostic, Node, Span } from 'aozora-wasm';

import { Document, ensureWasmReady } from './wasm-loader';

const diagnosticMessages: Readonly<Record<string, LocalizedText>> = {
  source_contains_pua: {
    ja: '私用領域文字がソースに紛れ込んでいます。',
    en: 'The source contains a private-use codepoint.',
  },
  unclosed_bracket: {
    ja: '開き括弧が閉じられていません。',
    en: 'An opening bracket is not closed.',
  },
  unmatched_close: {
    ja: '閉じ括弧に対応する開き括弧がありません。',
    en: 'A close bracket has no matching open bracket.',
  },
  accent_decomposition_applied: {
    ja: 'アクセント分解を適用しました。',
    en: 'Accent decomposition was applied.',
  },
  unresolved_gaiji: {
    ja: '外字参照を解決できませんでした。',
    en: 'The gaiji reference could not be resolved.',
  },
  mismatched_container_close: {
    ja: 'コンテナが異なる種別の閉じ注記で閉じられています。',
    en: 'The container is closed by a different kind.',
  },
  empty_ruby_reading: {
    ja: 'ルビの読みが空です。',
    en: 'The ruby reading is empty.',
  },
  nested_ruby: {
    ja: 'ルビの読みの中に別のルビがあります。',
    en: 'A ruby is nested inside another reading.',
  },
  unrecognised_container_directive: {
    ja: '未知のコンテナ注記です。',
    en: 'The container directive is not recognised.',
  },
  tcy_target_not_found: {
    ja: '縦中横の対象が前方に見つかりません。',
    en: 'The tate-chu-yoko target was not found.',
  },
  bouten_target_ambiguous: {
    ja: '傍点の対象が複数あり曖昧です。',
    en: 'The emphasis-dot target is ambiguous.',
  },
  forward_referent_not_stylable: {
    ja: '前方参照の対象をその場で装飾できません。',
    en: 'The forward-reference target cannot be styled in place.',
  },
  break_in_single_line_container: {
    ja: '単一行コンテナ内に改ページまたは改段があります。',
    en: 'A page or section break occurs inside a single-line container.',
  },
  bracketed_kaeriten_no_pair: {
    ja: '角括弧返り点に対応する基点がありません。',
    en: 'The bracketed kaeriten has no matching base.',
  },
  kaeriten_outside_kanbun: {
    ja: '返り点が漢文文脈の外にあります。',
    en: 'The kaeriten occurs outside a kanbun context.',
  },
  mismatched_bouten_container: {
    ja: '傍点と傍線の開閉が食い違っています。',
    en: 'The emphasis range is opened and closed by different families.',
  },
  non_canonical_directive: {
    ja: '注記が正規の綴りではありません。',
    en: 'The annotation uses a non-canonical spelling.',
  },
  residual_annotation_marker: {
    ja: '分類できない注記が残っています。',
    en: 'An annotation marker could not be classified.',
  },
  unregistered_sentinel: {
    ja: 'パーサ内部で未登録の記号を検出しました。',
    en: 'The parser encountered an unregistered internal sentinel.',
  },
  registry_out_of_order: {
    ja: 'パーサ内部のレジストリ順序が壊れています。',
    en: 'The parser’s internal registry is out of order.',
  },
  registry_position_mismatch: {
    ja: 'パーサ内部のレジストリ位置が一致しません。',
    en: 'The parser’s internal registry position does not match.',
  },
};

function byteToUtf16Table(source: string): Uint32Array {
  const byteLength = new TextEncoder().encode(source).length;
  const table = new Uint32Array(byteLength + 1);
  let byteOffset = 0;
  for (let utf16Offset = 0; utf16Offset < source.length; utf16Offset++) {
    const codePoint = source.codePointAt(utf16Offset);
    if (codePoint === undefined) break;
    const width =
      codePoint <= 0x7f
        ? 1
        : codePoint <= 0x7ff
          ? 2
          : codePoint <= 0xffff
            ? 3
            : 4;
    for (let index = 0; index < width; index++) {
      table[byteOffset + index] = utf16Offset;
    }
    byteOffset += width;
    if (codePoint > 0xffff) utf16Offset++;
  }
  table[byteOffset] = source.length;
  return table;
}

function toUtf16Range(table: Uint32Array, span: Span): TextRange {
  const last = table.length - 1;
  const start = table[Math.min(span.start, last)] ?? 0;
  const end = table[Math.min(span.end, last)] ?? start;
  return { start, end: Math.max(start, end) };
}

function diagnosticCode(entry: Diagnostic): string {
  const namespace = entry.kind === 'non_canonical_directive' ? 'lint' : 'lex';
  return `aozora::${namespace}::${entry.kind}`;
}

export function normalizeDiagnostics(
  source: string,
  diagnostics: readonly Diagnostic[],
): readonly PlaygroundDiagnostic[] {
  return normalizeDiagnosticsWithTable(diagnostics, byteToUtf16Table(source));
}

function normalizeDiagnosticsWithTable(
  diagnostics: readonly Diagnostic[],
  table: Uint32Array,
): readonly PlaygroundDiagnostic[] {
  return diagnostics.map((entry) => ({
    severity: entry.severity === 'note' ? 'info' : entry.severity,
    message: diagnosticMessages[entry.kind] ?? {
      ja: `診断: ${entry.kind.replaceAll('_', ' ')}`,
      en: entry.kind.replaceAll('_', ' '),
    },
    range: toUtf16Range(table, entry.span),
    code: diagnosticCode(entry),
  }));
}

function headingLevel(value: string): number {
  if (value.includes('中見出し')) return 2;
  if (value.includes('小見出し')) return 3;
  return 1;
}

function trimRange(
  source: string,
  start: number,
  end: number,
): TextRange | null {
  let trimmedStart = start;
  let trimmedEnd = end;
  while (trimmedStart < trimmedEnd && /\s/u.test(source[trimmedStart] ?? '')) {
    trimmedStart++;
  }
  while (
    trimmedEnd > trimmedStart &&
    /\s/u.test(source[trimmedEnd - 1] ?? '')
  ) {
    trimmedEnd--;
  }
  return trimmedStart < trimmedEnd
    ? { start: trimmedStart, end: trimmedEnd }
    : null;
}

function lineAfter(source: string, offset: number): TextRange | null {
  const start = source[offset] === '\n' ? offset + 1 : offset;
  const end = source.indexOf('\n', start);
  return trimRange(source, start, end === -1 ? source.length : end);
}

function textEntry(
  source: string,
  range: TextRange,
  level: number,
): PlaygroundOutlineEntry | null {
  const text = source
    .slice(range.start, range.end)
    .replace(/［＃[^］]*］/gu, '')
    .trim();
  return text ? { level, text, range } : null;
}

export function deriveOutline(
  source: string,
  nodes: readonly Node[],
  pairs: readonly ContainerPair[],
): readonly PlaygroundOutlineEntry[] {
  return deriveOutlineWithTable(source, nodes, pairs, byteToUtf16Table(source));
}

function deriveOutlineWithTable(
  source: string,
  nodes: readonly Node[],
  pairs: readonly ContainerPair[],
  table: Uint32Array,
): readonly PlaygroundOutlineEntry[] {
  const entries: PlaygroundOutlineEntry[] = [];

  for (const pair of pairs) {
    if (pair.kind !== 'heading') continue;
    const open = toUtf16Range(table, pair.open);
    const close = toUtf16Range(table, pair.close);
    const newline = source.indexOf('\n', open.end);
    const range = trimRange(
      source,
      newline === -1 ? open.end : newline + 1,
      close.start,
    );
    if (!range) continue;
    const entry = textEntry(
      source,
      range,
      headingLevel(source.slice(open.start, open.end)),
    );
    if (entry) entries.push(entry);
  }

  for (const node of nodes) {
    if (node.kind === 'heading') {
      const range = toUtf16Range(table, node.span);
      const value = source.slice(range.start, range.end);
      const annotation = value.indexOf('［＃');
      const contentEnd =
        annotation === -1 ? range.end : range.start + annotation;
      const content = trimRange(source, range.start, contentEnd);
      if (!content) continue;
      const lineStart = source.lastIndexOf('\n', content.end - 1) + 1;
      const line = trimRange(
        source,
        Math.max(content.start, lineStart),
        content.end,
      );
      if (!line) continue;
      const entry = textEntry(source, line, headingLevel(value));
      if (entry) entries.push(entry);
    } else if (node.kind === 'headingHint') {
      const hint = toUtf16Range(table, node.span);
      const value = source.slice(hint.start, hint.end);
      const target = /「([^」]+)」/u.exec(value)?.[1];
      if (!target) continue;
      const start = source.lastIndexOf(target, hint.start);
      if (start === -1) continue;
      const range = { start, end: start + target.length };
      const entry = textEntry(source, range, headingLevel(value));
      if (entry) entries.push(entry);
    } else if (node.kind === 'containerOpen') {
      const marker = toUtf16Range(table, node.span);
      const value = source.slice(marker.start, marker.end);
      if (!value.includes('見出し') || value.includes('終わり')) continue;
      const range = lineAfter(source, marker.end);
      if (!range) continue;
      const entry = textEntry(source, range, headingLevel(value));
      if (entry) entries.push(entry);
    }
  }

  const unique = new Map<string, PlaygroundOutlineEntry>();
  for (const entry of entries) {
    if (!entry.range) continue;
    unique.set(`${entry.range.start}:${entry.range.end}`, entry);
  }
  return [...unique.values()].sort(
    (left, right) => (left.range?.start ?? 0) - (right.range?.start ?? 0),
  );
}

export async function initializeEngine(): Promise<void> {
  await ensureWasmReady();
}

export async function analyze(
  source: string,
  context: AnalyzeContext,
): Promise<PlaygroundAnalysis> {
  if (context.signal.aborted) throw new DOMException('Aborted', 'AbortError');
  const document = new Document(source);
  try {
    const html = document.toHtml();
    const table = byteToUtf16Table(source);
    const diagnostics = normalizeDiagnosticsWithTable(
      document.diagnostics(),
      table,
    );
    const outline = deriveOutlineWithTable(
      source,
      document.nodes(),
      document.containerPairs(),
      table,
    );
    if (context.signal.aborted) {
      throw new DOMException('Aborted', 'AbortError');
    }
    return { html, diagnostics, outline };
  } finally {
    document.free();
  }
}
