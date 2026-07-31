import { describe, expect, it } from 'vitest';

import { message, messageKeys } from './catalog';
import type { Locale } from './types';

const locales = ['ja', 'en'] as const satisfies readonly Locale[];
const placeholderPattern = /\{(\w+)\}/g;

function placeholders(value: string): string[] {
  return [...value.matchAll(placeholderPattern)]
    .map((match) => match[1])
    .sort();
}

describe('message catalog', () => {
  it.each(locales)('contains a non-empty %s translation for every message', (locale) => {
    for (const key of messageKeys) {
      expect(message(locale, key).trim(), key).not.toBe('');
    }
  });

  it('keeps interpolation placeholders aligned across locales', () => {
    for (const key of messageKeys) {
      expect(placeholders(message('en', key)), key).toEqual(
        placeholders(message('ja', key)),
      );
    }
  });
});
