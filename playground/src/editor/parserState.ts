import { StateField } from '@codemirror/state';
import type { EditorState, Transaction } from '@codemirror/state';
import type {
  Diagnostic,
  GaijiResolution,
  Node,
  Pair,
  TextEdit as WasmTextEdit,
} from 'aozora-wasm';
import { Document } from '../wasm-loader';

/** A single container fold range, both endpoints in UTF-16 code units. */
export interface ContainerFold {
  /** End of the line carrying the open marker — fold starts here. */
  openLineEnd: number;
  /** Start of the close marker — fold ends here. */
  closeStart: number;
}

/** A heading entry surfaced to the outline panel. */
export interface HeadingEntry {
  /** UTF-16 code unit offset of the start of the heading span. */
  from: number;
  /** UTF-16 code unit offset of the end of the heading span. */
  to: number;
  /** Visible text of the heading (sliced from source). */
  text: string;
  /** Heading rank from the 大/中/小見出し hint (1/2/3); 1 when no hint.
   * Annotation-driven, not container-nesting depth. */
  level: number;
}

export type NodeEntry = Node;
export type DiagnosticEntry = Diagnostic;
export type PairEntry = Pair;
export type GaijiResolutionEntry = GaijiResolution;

export interface ParserState {
  doc: Document | null;
  source: string;
  html: string;
  serialized: string;
  /** Raw wire-format JSON kept for the nodes side tab. */
  nodesJson: string;
  nodes: NodeEntry[];
  diagnostics: DiagnosticEntry[];
  pairs: PairEntry[];
  gaijiResolutions: GaijiResolutionEntry[];
  parseDurationMs: number;
  byteLen: number;
  /** index = UTF-16 code unit offset, value = UTF-8 byte offset. Length = source.length + 1. */
  u2b: Uint32Array;
  /** index = UTF-8 byte offset, value = UTF-16 code unit offset. Length = byteLen + 1. */
  b2u: Uint32Array;
  /** Container open/close fold ranges, pre-computed once per parse. */
  containerFolds: ContainerFold[];
  /** Heading entries for the outline panel, in source order. */
  headings: HeadingEntry[];
}

/**
 * Build UTF-16 ↔ UTF-8 offset translation tables for `source`.
 *
 * `u2b[i]` is the UTF-8 byte offset where the i-th UTF-16 code unit
 * starts. `b2u[j]` is the UTF-16 code unit index for the character
 * that contains byte j. Both arrays include a sentinel terminator
 * (`u2b[source.length]` = total bytes, `b2u[totalBytes]` = source.length).
 *
 * Surrogate-pair accounting: the high surrogate "owns" all 4 bytes
 * of the encoded astral character; the low surrogate contributes 0.
 * That keeps `u2b` monotonically non-decreasing and lets `b2u`
 * resolve any interior byte to the starting UTF-16 index.
 *
 * Complexity: O(n) on source length, with one Uint32Array of size
 * `source.length + 1` and one of `byteLen + 1`. Memory: at most
 * `8 * (n_utf16 + n_byte)` bytes ≈ 8 × 2 × n_byte for typical
 * Japanese text. A 6 MB document is roughly 100 MB of table — large
 * but acceptable for an in-tab editor; smaller documents are
 * proportionally faster. Building the tables for a 6 MB doc takes
 * ~50 ms on a modern laptop.
 */
export function buildOffsetTables(source: string): {
  u2b: Uint32Array;
  b2u: Uint32Array;
  byteLen: number;
} {
  const len = source.length;
  const u2b = new Uint32Array(len + 1);
  let byte = 0;
  for (let i = 0; i < len; i++) {
    u2b[i] = byte;
    const code = source.charCodeAt(i);
    if (code < 0x80) byte += 1;
    else if (code < 0x800) byte += 2;
    else if (code >= 0xd800 && code < 0xdc00) byte += 4;
    else if (code >= 0xdc00 && code < 0xe000) byte += 0;
    else byte += 3;
  }
  u2b[len] = byte;
  const b2u = new Uint32Array(byte + 1);
  let utf16 = 0;
  for (let b = 0; b <= byte; b++) {
    while (utf16 < len && u2b[utf16 + 1] <= b && (utf16 + 1 < len || u2b[utf16 + 1] === b)) {
      utf16++;
    }
    b2u[b] = utf16;
  }
  return { u2b, b2u, byteLen: byte };
}

export interface ParseCallbacks {
  /** Called after every successful parse (source change). */
  onParse?: (payload: ParserState) => void;
}

let callbacks: ParseCallbacks = {};

export function setParseCallbacks(cb: ParseCallbacks): void {
  callbacks = cb;
}

interface SecondaryData {
  containerFolds: ContainerFold[];
  headings: HeadingEntry[];
}

function deriveSecondaryData(
  source: string,
  nodes: NodeEntry[],
  b2u: Uint32Array,
): SecondaryData {
  const containerFolds: ContainerFold[] = [];
  const headings: HeadingEntry[] = [];
  const stack: NodeEntry[] = [];

  const utf16At = (b: number): number => {
    if (b < 0) return 0;
    if (b >= b2u.length) return b2u[b2u.length - 1] ?? 0;
    return b2u[b] ?? 0;
  };

  /**
   * `headingHint` 注記から見出しレベルを推定する。
   * 大見出し → 1、中見出し → 2、小見出し → 3、その他 → 1。
   * `headingHint` は `aozoraHeading` の前（block 形式）か後（forward-reference 形式）に
   * 出現するので、両方のケースで「直近の aozoraHeading」に level を適用する。
   */
  function levelFromHint(hintText: string): number {
    if (hintText.includes('大見出し')) return 1;
    if (hintText.includes('中見出し')) return 2;
    if (hintText.includes('小見出し')) return 3;
    return 1;
  }

  let pendingLevel: number | null = null;

  for (const entry of nodes) {
    if (entry.kind === 'containerOpen') {
      stack.push(entry);
    } else if (entry.kind === 'containerClose') {
      const opened = stack.pop();
      if (!opened) continue;
      const openEndU16 = utf16At(opened.span.end);
      const closeStartU16 = utf16At(entry.span.start);
      // Find the end of the line carrying the open marker. Without
      // an EditorState in scope we approximate by looking forward
      // for the next newline in `source`.
      const nlIdx = source.indexOf('\n', openEndU16);
      const lineEnd = nlIdx === -1 ? openEndU16 : nlIdx;
      if (closeStartU16 > lineEnd) {
        containerFolds.push({ openLineEnd: lineEnd, closeStart: closeStartU16 });
      }
    } else if (entry.kind === 'headingHint') {
      const from = utf16At(entry.span.start);
      const to = utf16At(entry.span.end);
      const text = source.slice(from, to);
      const lvl = levelFromHint(text);
      // 直前の aozoraHeading にも level を遡及適用（forward-reference 形式）
      const prev = headings[headings.length - 1];
      if (prev && prev.level === 1) {
        prev.level = lvl;
      }
      // 次に来る aozoraHeading 用にも pending（block 形式）
      pendingLevel = lvl;
    } else if (entry.kind === 'heading') {
      const from = utf16At(entry.span.start);
      const to = utf16At(entry.span.end);
      const text = source.slice(from, to).trim();
      if (text.length > 0) {
        headings.push({ from, to, text, level: pendingLevel ?? 1 });
      }
      pendingLevel = null;
    }
  }
  return { containerFolds, headings };
}

function computeParserState(
  prev: ParserState | null,
  source: string,
  edits: WasmTextEdit[] | null,
): ParserState {
  const t0 = performance.now();
  let doc: Document;
  if (prev?.doc && edits) {
    doc = prev.doc;
    try {
      doc.edit(edits);
    } catch {
      doc.free();
      doc = new Document(source);
    }
  } else {
    prev?.doc?.free();
    doc = new Document(source);
  }
  const html = doc.toHtml();
  const parseDurationMs = performance.now() - t0;
  const serialized = doc.toSource();
  const nodes = Array.from(doc.nodes());
  const nodesJson = JSON.stringify(nodes, null, 2);
  const diagnostics = Array.from(doc.diagnostics());
  const pairs = Array.from(doc.pairs());
  const gaijiResolutions = Array.from(doc.gaiji());
  const byteLen = doc.sourceByteLen();
  const tables = buildOffsetTables(source);

  const secondary = deriveSecondaryData(source, nodes, tables.b2u);
  const ps: ParserState = {
    doc,
    source,
    html,
    serialized,
    nodesJson,
    nodes,
    diagnostics,
    pairs,
    gaijiResolutions,
    parseDurationMs,
    byteLen,
    u2b: tables.u2b,
    b2u: tables.b2u,
    containerFolds: secondary.containerFolds,
    headings: secondary.headings,
  };
  callbacks.onParse?.(ps);
  return ps;
}

/**
 * The single Document owner. All CM6 extensions read parsed data
 * from `view.state.field(parserStateField)`.
 */
export const parserStateField = StateField.define<ParserState>({
  create(state: EditorState): ParserState {
    return computeParserState(null, state.doc.toString(), null);
  },
  update(value: ParserState, tr: Transaction): ParserState {
    if (!tr.docChanged) return value;
    const edits: WasmTextEdit[] = [];
    tr.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
      edits.push({
        start: utf16ToByte(value, fromA),
        end: utf16ToByte(value, toA),
        replacement: inserted.toString(),
      });
    });
    return computeParserState(value, tr.newDoc.toString(), edits);
  },
});

/** UTF-16 code unit offset → UTF-8 byte offset (clamped). */
export function utf16ToByte(ps: ParserState, u16: number): number {
  if (u16 < 0) return 0;
  if (u16 >= ps.u2b.length) return ps.u2b[ps.u2b.length - 1] ?? 0;
  return ps.u2b[u16] ?? 0;
}

/** UTF-8 byte offset → UTF-16 code unit offset (clamped). */
export function byteToUtf16(ps: ParserState, byte: number): number {
  if (byte < 0) return 0;
  if (byte >= ps.b2u.length) return ps.b2u[ps.b2u.length - 1] ?? 0;
  return ps.b2u[byte] ?? 0;
}
