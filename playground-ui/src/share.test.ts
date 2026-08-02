import { compressToBase64 } from 'lz-string';
import { describe, expect, it, vi } from 'vitest';

import {
  copyShareUrl,
  encodeLegacyTextForTest,
  encodeSourceHash,
  readSharedSource,
  SHARE_SOURCE_LIMIT,
  SHARE_URL_LIMIT,
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

  it('reads an intentionally empty legacy compressed document', () => {
    const url = new URL('https://example.test/');
    url.searchParams.set('c', toBase64Url(compressToBase64('')));
    expect(readSharedSource(url)).toEqual({ status: 'ok', source: '' });

    const paddedUrl = new URL('https://example.test/');
    paddedUrl.searchParams.set('c', compressToBase64(''));
    expect(readSharedSource(paddedUrl)).toEqual({ status: 'ok', source: '' });
  });

  it('rejects invalid legacy UTF-8 and malformed empty data', () => {
    const invalidText = new URL('https://example.test/?text=_w');
    expect(readSharedSource(invalidText)).toEqual({ status: 'invalid' });

    for (const value of ['A', '!', 'AAAA', 'Q====']) {
      expect(
        readSharedSource(new URL(`https://example.test/?c=${value}`)),
      ).toEqual({ status: 'invalid' });
    }
  });

  it('applies the encoded input limit to every shared format', () => {
    const oversized = 'A'.repeat(SHARE_URL_LIMIT + 1);
    for (const url of [
      new URL(`https://example.test/#src=${oversized}`),
      new URL(`https://example.test/?text=${oversized}`),
      new URL(`https://example.test/?c=${oversized}`),
    ]) {
      expect(readSharedSource(url)).toEqual({ status: 'invalid' });
    }
  });

  it('applies the decoded source limit to every shared format', () => {
    const oversizedSource = 'a'.repeat(SHARE_SOURCE_LIMIT + 1);
    const hashUrl = new URL(
      `https://example.test/${encodeSourceHash(oversizedSource)}`,
    );
    expect(readSharedSource(hashUrl)).toEqual({ status: 'invalid' });

    const textUrl = new URL('https://example.test/');
    textUrl.searchParams.set('text', encodeLegacyTextForTest(oversizedSource));
    expect(readSharedSource(textUrl)).toEqual({ status: 'invalid' });

    const compressedUrl = new URL('https://example.test/');
    compressedUrl.searchParams.set(
      'c',
      toBase64Url(compressToBase64(oversizedSource)),
    );
    expect(readSharedSource(compressedUrl)).toEqual({ status: 'invalid' });
  });

  it('keeps the decoded source limit independent from the URL limit', () => {
    const compatibleSource = 'a'.repeat(100_000);
    const compatible = new URL('https://example.test/');
    compatible.searchParams.set(
      'c',
      toBase64Url(compressToBase64(compatibleSource)),
    );
    expect(readSharedSource(compatible)).toEqual({
      status: 'ok',
      source: compatibleSource,
    });
  });

  it('rejects malformed shared state without throwing', () => {
    const compressed = encodeSourceHash('hello');
    for (const url of [
      new URL('https://example.test/#src=%%%'),
      new URL(`https://example.test/${compressed}!`),
      new URL(`https://example.test/${compressed}A`),
      new URL('https://example.test/?c=BYUwNmD2Q===!'),
      new URL('https://example.test/?c=BYUwNmD2Q==A'),
    ]) {
      expect(readSharedSource(url)).toEqual({ status: 'invalid' });
    }
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

  it('rejects an oversized compressible source before changing history or clipboard', async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    history.replaceState(null, '', '/before');

    await expect(
      copyShareUrl('a'.repeat(SHARE_SOURCE_LIMIT + 1)),
    ).rejects.toBeInstanceOf(ShareUrlTooLongError);
    expect(location.pathname).toBe('/before');
    expect(location.hash).toBe('');
    expect(writeText).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
