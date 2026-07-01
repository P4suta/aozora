import { createSignal } from 'solid-js';
import { loadString, saveString } from '../storage';
import { CATALOG, type MessageKey } from './catalog';
import type { Locale } from './types';

// i18n core (#336 D-6). Mirrors `theme.ts` (pure load/save/apply + a module
// `bootstrapLang()`), but the reactive signal is module-level so every `t()` in
// JSX re-renders app-wide on a language switch, and CM6 extensions that call
// `t()` per keystroke (completion / linter) pick up the current locale too.

export type { Locale, MessageKey };

const STORAGE_KEY = 'locale';
export const LOCALES: readonly Locale[] = ['ja', 'en'];

function isLocale(v: string | null): v is Locale {
  return v === 'ja' || v === 'en';
}

function fromQuery(): Locale | null {
  const p = new URLSearchParams(window.location.search).get('lang');
  return isLocale(p) ? p : null;
}

function fromStorage(): Locale | null {
  return isLocale(loadString(STORAGE_KEY)) ? (loadString(STORAGE_KEY) as Locale) : null;
}

function fromNavigator(): Locale {
  return (navigator.language ?? 'ja').toLowerCase().startsWith('en') ? 'en' : 'ja';
}

const [locale, setLocaleSignal] = createSignal<Locale>('ja');
export { locale };

/** Set `<html lang>` so the document + assistive tech track the UI language. */
export function applyLang(l: Locale): void {
  document.documentElement.setAttribute('lang', l);
}

/** Switch language: update the signal, persist, and re-tag the document. */
export function setLocale(l: Locale): void {
  setLocaleSignal(l);
  saveString(STORAGE_KEY, l);
  applyLang(l);
}

/** Boot priority: `?lang=` → localStorage → `navigator.language` → `ja`. */
export function bootstrapLang(): void {
  const q = fromQuery();
  const initial = q ?? fromStorage() ?? fromNavigator();
  setLocaleSignal(initial);
  // A query-derived choice is persisted so a later visit without `?lang` keeps it.
  if (q) saveString(STORAGE_KEY, initial);
  applyLang(initial);
}

/** Translate a message key for the current locale (reactive via `locale()`). */
export function t(key: MessageKey): string {
  return CATALOG[locale()][key];
}

/** Like [`t`] but fills `{name}` placeholders from `params`. */
export function tf(key: MessageKey, params: Record<string, string | number>): string {
  return t(key).replace(/\{(\w+)\}/g, (_m, name: string) =>
    name in params ? String(params[name]) : `{${name}}`,
  );
}
