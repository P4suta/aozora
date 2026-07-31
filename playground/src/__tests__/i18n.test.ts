import { describe, expect, it } from 'vitest';
import { CATALOG } from '../i18n/catalog';

const placeholders = (s: string): string[] =>
  (s.match(/\{(\w+)\}/g) ?? []).sort();

describe('i18n catalog', () => {
  it('has the same keys in the Japanese and English catalogs', () => {
    expect(Object.keys(CATALOG.en).sort()).toEqual(
      Object.keys(CATALOG.ja).sort(),
    );
  });

  it('has a non-empty value for every key in both locales', () => {
    for (const locale of ['ja', 'en'] as const) {
      for (const [key, val] of Object.entries(CATALOG[locale])) {
        expect(val, `${locale}.${key}`).toBeTruthy();
      }
    }
  });

  it('keeps placeholders aligned between locales', () => {
    for (const key of Object.keys(CATALOG.ja) as Array<
      keyof typeof CATALOG.ja
    >) {
      expect(placeholders(CATALOG.en[key]), key).toEqual(
        placeholders(CATALOG.ja[key]),
      );
    }
  });
});
