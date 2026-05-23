/**
 * localStorage 永続化レイヤ。`?text=` 共有とは独立して、ブラウザに
 * 「最後に編集していた source」やエディタ設定を保存する。
 *
 * 設計メモ：
 * - サイズ上限は browser 仕様で約 5–10 MB。aozora source は通常 KB
 *   オーダーなので大半は問題ない。クォータ超過時の戻り値は `false`、
 *   呼び出し側でトースト等のフィードバックを出せる。
 * - 起動時の優先順位は呼び出し側で決める：`?text=` > localStorage >
 *   デフォルトサンプル、が現在の方針（App.tsx）。
 * - サブドメインを跨いで読み書きはできないので、PR プレビュー環境を
 *   作っても本番と source は混ざらない。
 */

import { warn } from './logger';

const SOURCE_KEY = 'aozora-playground:source:v1';
const KEY_PREFIX = 'aozora-playground:';

/** 既存の `source` 用 API は public ラッパで維持。 */
export function loadStoredSource(): string | null {
  return loadString(SOURCE_KEY);
}

/** 戻り値: 保存成功 → true、容量不足など失敗時 → false。 */
export function saveSource(text: string): boolean {
  if (text === '') {
    removeKey(SOURCE_KEY);
    return true;
  }
  return saveString(SOURCE_KEY, text);
}

export function clearStoredSource(): void {
  removeKey(SOURCE_KEY);
}

// ---------------- generic keyed helpers ----------------

/**
 * 任意の小さな永続化に使う汎用ヘルパ。複数フィールド（active tab,
 * theme preference, settings flags, etc.）を `aozora-playground:` 名前空間
 * の下に保存する。値はすべて文字列。
 */
export function loadString(key: string): string | null {
  try {
    return localStorage.getItem(normalize(key));
  } catch (err) {
    warn('loadString failed:', err);
    return null;
  }
}

export function saveString(key: string, value: string): boolean {
  try {
    localStorage.setItem(normalize(key), value);
    return true;
  } catch (err) {
    warn('saveString failed:', err);
    return false;
  }
}

export function removeKey(key: string): void {
  try {
    localStorage.removeItem(normalize(key));
  } catch (err) {
    warn('removeKey failed:', err);
  }
}

/** タブの index 等、数値 1 件用の薄いラッパ。 */
export function loadNumber(key: string): number | null {
  const raw = loadString(key);
  if (raw === null) return null;
  const n = Number.parseInt(raw, 10);
  return Number.isFinite(n) ? n : null;
}

export function saveNumber(key: string, value: number): boolean {
  return saveString(key, String(value));
}

function normalize(key: string): string {
  return key.startsWith(KEY_PREFIX) ? key : `${KEY_PREFIX}${key}`;
}
