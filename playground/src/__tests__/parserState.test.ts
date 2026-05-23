import { describe, expect, it } from 'vitest';
import { buildOffsetTables } from '../editor/parserState';

/**
 * buildOffsetTables の意味的不変条件：
 *   - u2b は monotonically non-decreasing
 *   - u2b[source.length] === source の UTF-8 byte length
 *   - b2u[byte] は byte を含む文字の先頭 UTF-16 index を返す
 *   - サロゲートペアは high が 4 バイトを「保有」、low は 0 バイト
 */
describe('buildOffsetTables', () => {
  function utf8ByteLen(s: string): number {
    return new TextEncoder().encode(s).length;
  }

  it('空文字では u2b=[0], b2u=[0], byteLen=0', () => {
    const { u2b, b2u, byteLen } = buildOffsetTables('');
    expect(u2b).toEqual(new Uint32Array([0]));
    expect(b2u).toEqual(new Uint32Array([0]));
    expect(byteLen).toBe(0);
  });

  it('ASCII 文字列で u2b[i] === i, b2u[b] === b', () => {
    const src = 'hello';
    const { u2b, b2u, byteLen } = buildOffsetTables(src);
    expect(byteLen).toBe(5);
    for (let i = 0; i <= src.length; i++) expect(u2b[i]).toBe(i);
    for (let b = 0; b <= byteLen; b++) expect(b2u[b]).toBe(b);
  });

  it('日本語（3 バイト CJK）で正しい byte offset を返す', () => {
    const src = '青空';
    const { u2b, byteLen } = buildOffsetTables(src);
    expect(byteLen).toBe(utf8ByteLen(src));
    expect(byteLen).toBe(6); // 3 + 3
    expect(u2b[0]).toBe(0);
    expect(u2b[1]).toBe(3);
    expect(u2b[2]).toBe(6);
  });

  it('全角括弧と漢字の混合で byteLen が TextEncoder と一致', () => {
    const src = '｜青梅《おうめ》';
    const { byteLen } = buildOffsetTables(src);
    expect(byteLen).toBe(utf8ByteLen(src));
  });

  it('サロゲートペア（4 バイト）で high が 4 を、low が 0 を寄与', () => {
    // U+1F4DA "📚" は high+low の 2 UTF-16 code units、UTF-8 で 4 バイト
    const src = '本📚';
    const { u2b, byteLen } = buildOffsetTables(src);
    expect(byteLen).toBe(utf8ByteLen(src));
    expect(byteLen).toBe(7); // 本 = 3, 📚 = 4

    // src.length === 3 (本 + high + low)
    expect(src.length).toBe(3);
    expect(u2b[0]).toBe(0); // 本 start
    expect(u2b[1]).toBe(3); // 📚 high start
    expect(u2b[2]).toBe(7); // 📚 low → already past the 4 bytes
    expect(u2b[3]).toBe(7); // sentinel
  });

  it('u2b は monotonically non-decreasing', () => {
    const src = 'a青b梅c📚d';
    const { u2b } = buildOffsetTables(src);
    for (let i = 1; i < u2b.length; i++) {
      expect(u2b[i]!).toBeGreaterThanOrEqual(u2b[i - 1]!);
    }
  });

  it('b2u はバイト中点から正しい UTF-16 index を引ける', () => {
    const src = 'a青b';
    const { b2u } = buildOffsetTables(src);
    // byte 0 → 'a' (utf16 0)
    expect(b2u[0]).toBe(0);
    // byte 1,2,3 → '青' の途中（utf16 1 を返す）
    expect(b2u[1]).toBe(1);
    expect(b2u[2]).toBe(1);
    // byte 4 → 'b' (utf16 2)
    expect(b2u[4]).toBe(2);
  });
});
