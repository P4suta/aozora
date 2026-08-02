import LZString, { decompressFromBase64 } from 'lz-string';

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });
export const SHARE_URL_LIMIT = 3500;
export const SHARE_SOURCE_LIMIT = 1_000_000;

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

function normalizedBase64(value: string): string {
  if (!/^[A-Za-z0-9+/_-]*={0,3}$/u.test(value)) {
    throw new Error('legacy shared source is invalid');
  }
  const unpadded = value.replace(/=+$/u, '');
  return unpadded.replaceAll('-', '+').replaceAll('_', '/');
}

function requireSupportedEncodedLength(value: string): void {
  if (value.length > SHARE_URL_LIMIT) {
    throw new Error('encoded shared source is too large');
  }
}

function requireSupportedSourceLength(source: string): string {
  if (source.length > SHARE_SOURCE_LIMIT) {
    throw new Error('shared source is too large');
  }
  return source;
}

function decodeLegacyText(value: string): string {
  requireSupportedEncodedLength(value);
  const binary = atob(fromBase64Url(value));
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  return requireSupportedSourceLength(decoder.decode(bytes));
}

function decodeLegacyCompressed(value: string): string {
  requireSupportedEncodedLength(value);
  const normalized = normalizedBase64(value);
  const result = decompressFromBase64(fromBase64Url(value));
  if (result === null) throw new Error('legacy compressed source is invalid');
  requireSupportedSourceLength(result);
  if (normalizedBase64(LZString.compressToBase64(result)) !== normalized) {
    throw new Error('legacy compressed source is invalid');
  }
  return result;
}

export function encodeSourceHash(source: string): string {
  return `#src=${LZString.compressToEncodedURIComponent(source)}`;
}

function decodeSourceHash(hash: string): string | null {
  const params = new URLSearchParams(hash.replace(/^#/, ''));
  const encoded = params.get('src');
  if (encoded === null) return null;
  requireSupportedEncodedLength(encoded);
  const normalized = encoded.replaceAll(' ', '+');
  if (!/^[A-Za-z0-9+$-]+$/u.test(normalized)) {
    throw new Error('source hash is invalid');
  }
  const decoded = LZString.decompressFromEncodedURIComponent(normalized);
  if (decoded === null) {
    throw new Error('source hash is invalid');
  }
  requireSupportedSourceLength(decoded);
  if (LZString.compressToEncodedURIComponent(decoded) !== normalized) {
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
  if (source.length > SHARE_SOURCE_LIMIT) {
    throw new ShareUrlTooLongError();
  }
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
