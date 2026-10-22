import type { EditorState, Transaction } from '@codemirror/state';
import { StateField } from '@codemirror/state';
import type {
  Diagnostic,
  GaijiResolution,
  Node,
  Pair,
  TextEdit as WasmTextEdit,
} from 'aozora-wasm';
import { Document } from '../wasm-loader';

const UTF8_SINGLE_BYTE_LIMIT = 0x80;
const UTF8_TWO_BYTE_LIMIT = 0x800;
const UTF16_HIGH_SURROGATE_START = 0xd800;
const UTF16_LOW_SURROGATE_START = 0xdc00;
const UTF16_SURROGATE_END = 0xe000;
const UTF8_TWO_BYTE_WIDTH = 2;
const UTF8_THREE_BYTE_WIDTH = 3;
const UTF8_FOUR_BYTE_WIDTH = 4;

/** A container fold range in UTF-16 code units. */
export interface ContainerFold {
  /** Fold start at the end of the opening marker's line. */
  openLineEnd: number;
  /** Fold end at the start of the closing marker. */
  closeStart: number;
}

export type NodeEntry = Node;
export type DiagnosticEntry = Diagnostic;
export type PairEntry = Pair;
export type GaijiResolutionEntry = GaijiResolution;

export interface ParserState {
  doc: Document | null;
  source: string;
  nodes: NodeEntry[];
  diagnostics: DiagnosticEntry[];
  pairs: PairEntry[];
  gaijiResolutions: GaijiResolutionEntry[];
  /** UTF-16 offsets mapped to UTF-8 byte offsets, including the end offset. */
  u2b: Uint32Array;
  /** UTF-8 byte offsets mapped to UTF-16 offsets, including the end offset. */
  b2u: Uint32Array;
  containerFolds: ContainerFold[];
}

/**
 * Build UTF-16 and UTF-8 offset translation tables for `source`.
 *
 * Both tables include the end offset.
 * Interior bytes map to the character's first UTF-16 code unit.
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
    if (code < UTF8_SINGLE_BYTE_LIMIT) byte += 1;
    else if (code < UTF8_TWO_BYTE_LIMIT) byte += UTF8_TWO_BYTE_WIDTH;
    else if (
      code >= UTF16_HIGH_SURROGATE_START &&
      code < UTF16_LOW_SURROGATE_START
    ) {
      const next = source.charCodeAt(i + 1);
      byte +=
        next >= UTF16_LOW_SURROGATE_START && next < UTF16_SURROGATE_END
          ? UTF8_FOUR_BYTE_WIDTH
          : UTF8_THREE_BYTE_WIDTH;
    } else if (
      code >= UTF16_LOW_SURROGATE_START &&
      code < UTF16_SURROGATE_END
    ) {
      const previous = source.charCodeAt(i - 1);
      byte +=
        previous >= UTF16_HIGH_SURROGATE_START &&
        previous < UTF16_LOW_SURROGATE_START
          ? 0
          : UTF8_THREE_BYTE_WIDTH;
    } else byte += UTF8_THREE_BYTE_WIDTH;
  }
  u2b[len] = byte;
  const b2u = new Uint32Array(byte + 1);
  let utf16 = 0;
  for (let b = 0; b <= byte; b++) {
    while (
      utf16 < len &&
      u2b[utf16 + 1] <= b &&
      (utf16 + 1 < len || u2b[utf16 + 1] === b)
    ) {
      utf16++;
    }
    b2u[b] = utf16;
  }
  return { u2b, b2u, byteLen: byte };
}

function deriveContainerFolds(
  source: string,
  nodes: NodeEntry[],
  b2u: Uint32Array,
): ContainerFold[] {
  const containerFolds: ContainerFold[] = [];
  const stack: NodeEntry[] = [];

  const utf16At = (b: number): number => {
    if (b < 0) return 0;
    if (b >= b2u.length) return b2u[b2u.length - 1] ?? 0;
    return b2u[b] ?? 0;
  };

  for (const entry of nodes) {
    if (entry.kind === 'containerOpen') {
      stack.push(entry);
    } else if (entry.kind === 'containerClose') {
      const opened = stack.pop();
      if (!opened) continue;
      const openEndU16 = utf16At(opened.span.end);
      const closeStartU16 = utf16At(entry.span.start);
      const nlIdx = source.indexOf('\n', openEndU16);
      const lineEnd = nlIdx === -1 ? openEndU16 : nlIdx;
      if (closeStartU16 > lineEnd) {
        containerFolds.push({
          openLineEnd: lineEnd,
          closeStart: closeStartU16,
        });
      }
    }
  }
  return containerFolds;
}

function computeParserState(
  prev: ParserState | null,
  source: string,
  edits: WasmTextEdit[] | null,
): ParserState {
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
  const nodes = Array.from(doc.nodes());
  const diagnostics = Array.from(doc.diagnostics());
  const pairs = Array.from(doc.pairs());
  const gaijiResolutions = Array.from(doc.gaiji());
  const tables = buildOffsetTables(source);

  return {
    doc,
    source,
    nodes,
    diagnostics,
    pairs,
    gaijiResolutions,
    u2b: tables.u2b,
    b2u: tables.b2u,
    containerFolds: deriveContainerFolds(source, nodes, tables.b2u),
  };
}

/** Own the editor's parsed document and update its projections after each edit. */
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

/** Convert a UTF-16 code unit offset to a UTF-8 byte offset, clamping to the source bounds. */
export function utf16ToByte(ps: ParserState, u16: number): number {
  if (u16 < 0) return 0;
  if (u16 >= ps.u2b.length) return ps.u2b[ps.u2b.length - 1] ?? 0;
  return ps.u2b[u16] ?? 0;
}

/** Convert a UTF-8 byte offset to a UTF-16 code unit offset, clamping to the source bounds. */
export function byteToUtf16(ps: ParserState, byte: number): number {
  if (byte < 0) return 0;
  if (byte >= ps.b2u.length) return ps.b2u[ps.b2u.length - 1] ?? 0;
  return ps.b2u[byte] ?? 0;
}
