/**
 * Theme preference plumbing.
 *
 * - 「auto」: OS の `prefers-color-scheme` に追従。OS テーマ変化時に
 *   `data-theme` 属性を即時更新する
 * - 「light」/「dark」: 強制。OS 設定は無視
 *
 * Single source of truth は `<html data-theme="light" | "dark">`。
 * CSS variable はその attribute 経由でしか切り替えないので、JS の
 * 状態と DOM の見た目がズレない。
 */

import { loadString, saveString } from './storage';

export type ThemePref = 'auto' | 'light' | 'dark';

const STORAGE_KEY = 'theme';

export function loadThemePref(): ThemePref {
  const v = loadString(STORAGE_KEY);
  if (v === 'light' || v === 'dark' || v === 'auto') return v;
  return 'auto';
}

export function saveThemePref(pref: ThemePref): void {
  saveString(STORAGE_KEY, pref);
}

function osPrefersDark(): boolean {
  return window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false;
}

export function effectiveTheme(pref: ThemePref): 'light' | 'dark' {
  if (pref === 'auto') return osPrefersDark() ? 'dark' : 'light';
  return pref;
}

export function applyTheme(pref: ThemePref): void {
  document.documentElement.setAttribute('data-theme', effectiveTheme(pref));
}

/**
 * `main.tsx` の最上部で 1 回呼ぶ。
 *   1. 保存された preference を読み、`data-theme` を適切に設定
 *   2. `auto` モードのときに OS テーマ変化を購読
 *
 * FOUC を避けるため、Solid の render より前に呼ぶこと。
 */
export function bootstrapTheme(): void {
  applyTheme(loadThemePref());

  const mql = window.matchMedia?.('(prefers-color-scheme: dark)');
  if (mql) {
    mql.addEventListener('change', () => {
      if (loadThemePref() === 'auto') applyTheme('auto');
    });
  }
}
