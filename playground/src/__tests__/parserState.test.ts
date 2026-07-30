import { EditorState } from '@codemirror/state';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  buildOffsetTables,
  byteToUtf16,
  parserStateField,
  utf16ToByte,
} from '../editor/parserState';
import { Document } from '../wasm-loader';

/**
 * Semantic expectations for buildOffsetTables:
 *   - u2b is monotonically non-decreasing
 *   - u2b[source.length] equals the source UTF-8 byte length
 *   - b2u[byte] returns the first UTF-16 index of the character containing byte
 *   - the high surrogate owns all four bytes and the low surrogate owns none
 */
describe('buildOffsetTables', () => {
  function utf8ByteLen(s: string): number {
    return new TextEncoder().encode(s).length;
  }

  it('returns zeroed tables and length for an empty string', () => {
    const { u2b, b2u, byteLen } = buildOffsetTables('');
    expect(u2b).toEqual(new Uint32Array([0]));
    expect(b2u).toEqual(new Uint32Array([0]));
    expect(byteLen).toBe(0);
  });

  it('maps UTF-16 and byte offsets one-to-one for ASCII', () => {
    const src = 'hello';
    const { u2b, b2u, byteLen } = buildOffsetTables(src);
    expect(byteLen).toBe(5);
    for (let i = 0; i <= src.length; i++) expect(u2b[i]).toBe(i);
    for (let b = 0; b <= byteLen; b++) expect(b2u[b]).toBe(b);
  });

  it('returns correct byte offsets for three-byte CJK characters', () => {
    const src = '青空';
    const { u2b, byteLen } = buildOffsetTables(src);
    expect(byteLen).toBe(utf8ByteLen(src));
    expect(byteLen).toBe(6); // 3 + 3
    expect(u2b[0]).toBe(0);
    expect(u2b[1]).toBe(3);
    expect(u2b[2]).toBe(6);
  });

  it('returns correct offsets for two-byte characters', () => {
    const { u2b, b2u, byteLen } = buildOffsetTables('é');
    expect(byteLen).toBe(2);
    expect(u2b).toEqual(new Uint32Array([0, 2]));
    expect(b2u).toEqual(new Uint32Array([0, 0, 1]));
  });

  it('matches TextEncoder for full-width brackets mixed with kanji', () => {
    const src = '｜青梅《おうめ》';
    const { byteLen } = buildOffsetTables(src);
    expect(byteLen).toBe(utf8ByteLen(src));
  });

  it('assigns four bytes to the high surrogate and none to the low surrogate', () => {
    // U+1F4DA is two UTF-16 code units but four UTF-8 bytes.
    const src = '本📚';
    const { u2b, byteLen } = buildOffsetTables(src);
    expect(byteLen).toBe(utf8ByteLen(src));
    expect(byteLen).toBe(7);

    // The source consists of one BMP code unit plus the surrogate pair.
    expect(src.length).toBe(3);
    expect(u2b[0]).toBe(0);
    expect(u2b[1]).toBe(3); // 📚 high start
    expect(u2b[2]).toBe(7); // 📚 low → already past the 4 bytes
    expect(u2b[3]).toBe(7); // sentinel
  });

  it('keeps u2b monotonically non-decreasing', () => {
    const src = 'a青b梅c📚d';
    const { u2b } = buildOffsetTables(src);
    for (let i = 1; i < u2b.length; i++) {
      expect(u2b[i]!).toBeGreaterThanOrEqual(u2b[i - 1]!);
    }
  });

  it('maps bytes inside a multibyte character to its UTF-16 index', () => {
    const src = 'a青b';
    const { b2u } = buildOffsetTables(src);
    // byte 0 → 'a' (utf16 0)
    expect(b2u[0]).toBe(0);
    // Every byte within the CJK character maps to its leading UTF-16 index.
    expect(b2u[1]).toBe(1);
    expect(b2u[2]).toBe(1);
    // byte 4 → 'b' (utf16 2)
    expect(b2u[4]).toBe(2);
  });
});

describe('parserStateField', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('projects parser data and container folds', () => {
    const toHtml = vi.spyOn(Document.prototype, 'toHtml');
    const toSource = vi.spyOn(Document.prototype, 'toSource');
    const sourceByteLen = vi.spyOn(Document.prototype, 'sourceByteLen');
    const source = ['OPEN', 'folded', 'CLOSE', 'tail'].join('\n');
    const encoder = new TextEncoder();
    const span = (value: string) => {
      const start = source.indexOf(value);
      return {
        start: encoder.encode(source.slice(0, start)).length,
        end:
          encoder.encode(source.slice(0, start)).length +
          encoder.encode(value).length,
      };
    };
    const open = span('OPEN');
    const close = span('CLOSE');

    vi.spyOn(Document.prototype, 'nodes').mockReturnValue([
      { kind: 'containerClose', span: close },
      { kind: 'containerOpen', span: open },
      { kind: 'containerClose', span: close },
      { kind: 'containerOpen', span: close },
      { kind: 'containerClose', span: open },
      { kind: 'containerOpen', span: { start: -10, end: -1 } },
      {
        kind: 'containerClose',
        span: { start: Number.MAX_SAFE_INTEGER, end: Number.MAX_SAFE_INTEGER },
      },
    ]);

    const state = EditorState.create({
      doc: source,
      extensions: [parserStateField],
    }).field(parserStateField);

    expect(state.source).toBe(source);
    expect(state.nodes).toHaveLength(7);
    expect(state.diagnostics).toEqual([]);
    expect(state.pairs).toEqual([]);
    expect(state.gaijiResolutions).toEqual([]);
    expect(toHtml).not.toHaveBeenCalled();
    expect(toSource).not.toHaveBeenCalled();
    expect(sourceByteLen).not.toHaveBeenCalled();
    expect(state.containerFolds).toEqual([
      {
        openLineEnd: source.indexOf('\n'),
        closeStart: source.indexOf('CLOSE'),
      },
      {
        openLineEnd: source.indexOf('\n'),
        closeStart: source.length,
      },
    ]);
  });

  it('reuses incremental documents, skips unchanged transactions, and recovers from edit errors', () => {
    const edit = vi.spyOn(Document.prototype, 'edit');
    const free = vi.spyOn(Document.prototype, 'free');
    let editorState = EditorState.create({
      doc: 'a😀b',
      extensions: [parserStateField],
    });
    const initial = editorState.field(parserStateField);

    const unchanged = editorState.update({}).state;
    expect(unchanged.field(parserStateField)).toBe(initial);

    editorState = editorState.update({
      changes: { from: 1, to: 3, insert: 'é' },
    }).state;
    const incremented = editorState.field(parserStateField);
    expect(incremented.doc).toBe(initial.doc);
    expect(edit).toHaveBeenLastCalledWith([
      { start: 1, end: 5, replacement: 'é' },
    ]);

    edit.mockImplementationOnce(() => {
      throw new Error('incremental edit failed');
    });
    const previousDocument = incremented.doc;
    editorState = editorState.update({
      changes: { from: 0, to: 1, insert: 'x' },
    }).state;
    const recovered = editorState.field(parserStateField);
    expect(recovered.doc).not.toBe(previousDocument);
    expect(free.mock.instances).toContain(previousDocument);

    recovered.doc = null;
    editorState = editorState.update({
      changes: { from: 0, insert: 'z' },
    }).state;
    expect(editorState.field(parserStateField).doc).toBeInstanceOf(Document);
  });

  it('clamps UTF-16 and byte offsets at both boundaries', () => {
    const parserState = EditorState.create({
      doc: 'a青😀',
      extensions: [parserStateField],
    }).field(parserStateField);

    expect(utf16ToByte(parserState, -1)).toBe(0);
    expect(utf16ToByte(parserState, 1)).toBe(1);
    expect(utf16ToByte(parserState, 999)).toBe(
      parserState.u2b[parserState.u2b.length - 1],
    );
    expect(byteToUtf16(parserState, -1)).toBe(0);
    expect(byteToUtf16(parserState, 2)).toBe(1);
    expect(byteToUtf16(parserState, 999)).toBe(parserState.source.length);

    const emptyTables = {
      ...parserState,
      u2b: new Uint32Array(),
      b2u: new Uint32Array(),
    };
    expect(utf16ToByte(emptyTables, 0)).toBe(0);
    expect(byteToUtf16(emptyTables, 0)).toBe(0);
  });
});
