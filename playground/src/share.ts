import { compressToBase64, decompressFromBase64 } from 'lz-string';
import { warn } from './logger';

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function toBase64Url(b64: string): string {
  return b64.replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

function fromBase64Url(param: string): string {
  const b64 = param.replaceAll('-', '+').replaceAll('_', '/');
  return b64 + '='.repeat((4 - (b64.length % 4)) % 4);
}

export function textToParam(text: string): string {
  const bytes = encoder.encode(text);
  let bin = '';
  for (const b of bytes) bin += String.fromCharCode(b);
  return toBase64Url(btoa(bin));
}

export function paramToText(param: string): string {
  const bin = atob(fromBase64Url(param));
  const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
  return decoder.decode(bytes);
}

/**
 * lz-string 圧縮を `?text=` と同じ base64url 規律で符号化する。`+` / `/` / `=`
 * を含まないので `URLSearchParams` が percent-encode せず、URL がクリーンに保たれる。
 */
export function textToCompressedParam(text: string): string {
  return toBase64Url(compressToBase64(text));
}

export function compressedParamToText(param: string): string {
  const out = decompressFromBase64(fromBase64Url(param));
  // 非空テキストのみを圧縮符号化するため、空/`null` は malformed とみなす。
  if (!out) throw new Error('lz-string decompress failed');
  return out;
}

export const SHARE_URL_LIMIT = 3500;

/**
 * 共有パラメータ（`text` 生 or `c` 圧縮）のうち短い方を選ぶ。短文では生 base64url
 * が短く読みやすいまま、長文・反復の多い文では圧縮が勝つ。
 */
function chooseShareParam(text: string): { key: 'text' | 'c'; value: string } {
  const raw = textToParam(text);
  const compressed = textToCompressedParam(text);
  return compressed.length < raw.length
    ? { key: 'c', value: compressed }
    : { key: 'text', value: raw };
}

/**
 * `params` 上の共有パラメータだけを書き換え、`text` / `c` 以外（例 `?lang=`）は保全する。
 */
function applyShareParam(params: URLSearchParams, text: string): void {
  params.delete('text');
  params.delete('c');
  if (!text) return;
  const { key, value } = chooseShareParam(text);
  params.set(key, value);
}

export function buildShareUrl(text: string): { url: string; tooLong: boolean } {
  const params = new URLSearchParams(location.search);
  applyShareParam(params, text);
  const query = params.toString();
  const url = `${location.origin}${location.pathname}${query ? `?${query}` : ''}`;
  return { url, tooLong: url.length > SHARE_URL_LIMIT };
}

/**
 * 起動時の共有 URL 読み出し結果。`none` は param 自体なし、`ok` は正常デコード、
 * `invalid` はデコード失敗（malformed URL）。呼び出し側は `invalid` の時に
 * トーストで通知する。生 `?text=` を優先し（既存リンクの後方互換）、無ければ
 * 圧縮 `?c=` を読む。
 */
export type ShareUrlReadResult =
  | { status: 'none' }
  | { status: 'ok'; text: string }
  | { status: 'invalid' };

export function readShareTextFromUrl(): ShareUrlReadResult {
  const params = new URLSearchParams(location.search);
  const raw = params.get('text');
  const compressed = params.get('c');
  if (raw === null && compressed === null) return { status: 'none' };
  try {
    if (raw !== null) return { status: 'ok', text: paramToText(raw) };
    return { status: 'ok', text: compressedParamToText(compressed as string) };
  } catch (err) {
    warn('Failed to decode share param:', err);
    return { status: 'invalid' };
  }
}

export function syncTextToUrl(text: string): void {
  const params = new URLSearchParams(location.search);
  applyShareParam(params, text);
  // 上限超過時は共有パラメータだけ落とし、その他（`?lang=` 等）は残す。
  if (`${location.pathname}?${params.toString()}`.length > SHARE_URL_LIMIT) {
    params.delete('text');
    params.delete('c');
  }
  const query = params.toString();
  history.replaceState(null, '', query ? `${location.pathname}?${query}` : location.pathname);
}
