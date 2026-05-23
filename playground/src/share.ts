import { warn } from './logger';

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export function textToParam(text: string): string {
  const bytes = encoder.encode(text);
  let bin = '';
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

export function paramToText(param: string): string {
  const b64 = param.replaceAll('-', '+').replaceAll('_', '/');
  const padded = b64 + '='.repeat((4 - (b64.length % 4)) % 4);
  const bin = atob(padded);
  const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
  return decoder.decode(bytes);
}

export const SHARE_URL_LIMIT = 3500;

export function buildShareUrl(text: string): { url: string; tooLong: boolean } {
  const param = textToParam(text);
  const url = `${location.origin}${location.pathname}?text=${param}`;
  return { url, tooLong: url.length > SHARE_URL_LIMIT };
}

/**
 * 起動時の `?text=` URL 読み出し結果。`none` は param 自体なし、
 * `ok` は正常デコード、`invalid` はデコード失敗（malformed URL）。
 * 呼び出し側は `invalid` の時にトーストで通知する。
 */
export type ShareUrlReadResult =
  | { status: 'none' }
  | { status: 'ok'; text: string }
  | { status: 'invalid' };

export function readShareTextFromUrl(): ShareUrlReadResult {
  const params = new URLSearchParams(location.search);
  const raw = params.get('text');
  if (!raw) return { status: 'none' };
  try {
    return { status: 'ok', text: paramToText(raw) };
  } catch (err) {
    warn('Failed to decode ?text= param:', err);
    return { status: 'invalid' };
  }
}

export function syncTextToUrl(text: string): void {
  if (!text) {
    history.replaceState(null, '', `${location.pathname}`);
    return;
  }
  const param = textToParam(text);
  const next = `${location.pathname}?text=${param}`;
  if (next.length > SHARE_URL_LIMIT) {
    history.replaceState(null, '', `${location.pathname}`);
    return;
  }
  history.replaceState(null, '', next);
}
