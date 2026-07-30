import { compressToBase64 } from 'lz-string';
import { describe, expect, it, vi } from 'vitest';

import {
  copyShareUrl,
  encodeLegacyTextForTest,
  encodeSourceHash,
  readSharedSource,
  ShareUrlTooLongError,
} from './share';

function toBase64Url(value: string): string {
  return value.replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

describe('shared source compatibility', () => {
  it('round trips the explicit hash format', () => {
    const source = '# 見出し\n\n吾輩《わがはい》';
    const url = new URL(`https://example.test/${encodeSourceHash(source)}`);
    expect(readSharedSource(url)).toEqual({ status: 'ok', source });
  });

  it('round trips an intentionally empty document', () => {
    const url = new URL(`https://example.test/${encodeSourceHash('')}`);
    expect(readSharedSource(url)).toEqual({ status: 'ok', source: '' });
  });

  it('reads the legacy aozora text and compressed query formats', () => {
    const source = '青空｜文庫《ぶんこ》';
    const textUrl = new URL('https://example.test/');
    textUrl.searchParams.set('text', encodeLegacyTextForTest(source));
    expect(readSharedSource(textUrl)).toEqual({ status: 'ok', source });

    const compressedUrl = new URL('https://example.test/');
    compressedUrl.searchParams.set('c', toBase64Url(compressToBase64(source)));
    expect(readSharedSource(compressedUrl)).toEqual({ status: 'ok', source });
  });

  it('rejects malformed shared state without throwing', () => {
    expect(readSharedSource(new URL('https://example.test/#src=%%%'))).toEqual({
      status: 'invalid',
    });
  });

  it('gives the hash precedence over legacy query parameters', () => {
    const url = new URL(
      `https://example.test/?text=${encodeLegacyTextForTest('legacy')}${encodeSourceHash('current')}`,
    );
    expect(readSharedSource(url)).toEqual({
      status: 'ok',
      source: 'current',
    });
  });

  it('updates and copies the URL only during the explicit share operation', async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    history.replaceState(null, '', '/?text=legacy&c=legacy');

    await copyShareUrl('# shared');

    expect(location.search).toBe('');
    expect(location.hash).toMatch(/^#src=/);
    expect(writeText).toHaveBeenCalledWith(location.href);
    vi.unstubAllGlobals();
  });

  it('rejects an impractically long URL before changing history or clipboard', async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    history.replaceState(null, '', '/before');
    const source = Array.from(
      { length: 5000 },
      (_, index) => `${index.toString(36)}:${(index * 7919).toString(36)}`,
    ).join('\n');

    await expect(copyShareUrl(source)).rejects.toBeInstanceOf(
      ShareUrlTooLongError,
    );
    expect(location.pathname).toBe('/before');
    expect(location.hash).toBe('');
    expect(writeText).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
