import { CATALOG, type MessageKey } from './catalog';
import type { Locale } from './types';

export type { Locale, MessageKey };

let editorLocale: Locale = 'ja';

export function setEditorLocale(locale: Locale): void {
  editorLocale = locale;
}

export function t(key: MessageKey): string {
  return CATALOG[editorLocale][key];
}

export function tf(
  key: MessageKey,
  params: Readonly<Record<string, string | number>>,
): string {
  return t(key).replace(/\{(\w+)\}/g, (_match, name: string) =>
    name in params ? String(params[name]) : `{${name}}`,
  );
}
