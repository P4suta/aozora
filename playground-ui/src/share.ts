import LZString, { decompressFromBase64 } from 'lz-string';

const encoder = new TextEncoder();
const decoder = new TextDecoder();
export const SHARE_URL_LIMIT = 3500;

export class ShareUrlTooLongError extends Error {
  constructor() {
    super('share URL exceeds the supported length');
    this.name = 'ShareUrlTooLongError';
  }
}

export type SharedSourceResult =
  | { readonly status: 'none' }
  | { readonly status: 'ok'; readonly source: string }
  | { readonly status: 'invalid' };

function fromBase64Url(value: string): string {
  const base64 = value.replaceAll('-', '+').replaceAll('_', '/');
  return base64 + '='.repeat((4 - (base64.length % 4)) % 4);
}

function decodeLegacyText(value: string): string {
  const binary = atob(fromBase64Url(value));
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  return decoder.decode(bytes);
}

function decodeLegacyCompressed(value: string): string {
  const result = decompressFromBase64(fromBase64Url(value));
  if (!result) throw new Error('legacy compressed source is invalid');
  return result;
}

export function encodeSourceHash(source: string): string {
  return `#src=${LZString.compressToEncodedURIComponent(source)}`;
}

function decodeSourceHash(hash: string): string | null {
  const params = new URLSearchParams(hash.replace(/^#/, ''));
  const encoded = params.get('src');
  if (encoded === null) return null;
  const decoded = LZString.decompressFromEncodedURIComponent(encoded);
  if (
    decoded === null ||
    (decoded === '' && encoded !== LZString.compressToEncodedURIComponent(''))
  ) {
    throw new Error('source hash is invalid');
  }
  return decoded;
}

export function readSharedSource(url: URL): SharedSourceResult {
  try {
    const fromHash = decodeSourceHash(url.hash);
    if (fromHash !== null) return { status: 'ok', source: fromHash };

    const legacyText = url.searchParams.get('text');
    if (legacyText !== null) {
      return { status: 'ok', source: decodeLegacyText(legacyText) };
    }

    const legacyCompressed = url.searchParams.get('c');
    if (legacyCompressed !== null) {
      return {
        status: 'ok',
        source: decodeLegacyCompressed(legacyCompressed),
      };
    }
    return { status: 'none' };
  } catch {
    return { status: 'invalid' };
  }
}

export async function copyShareUrl(source: string): Promise<void> {
  const url = new URL(globalThis.location.href);
  url.searchParams.delete('text');
  url.searchParams.delete('c');
  url.hash = encodeSourceHash(source);
  if (url.toString().length > SHARE_URL_LIMIT) {
    throw new ShareUrlTooLongError();
  }
  globalThis.history.replaceState(null, '', url);
  await globalThis.navigator.clipboard.writeText(url.toString());
}

export function encodeLegacyTextForTest(source: string): string {
  const bytes = encoder.encode(source);
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary)
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replace(/=+$/, '');
}
