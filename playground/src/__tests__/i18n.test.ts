import { describe, expect, it } from 'vitest';
import { CATALOG } from '../i18n/catalog';

const placeholders = (s: string): string[] => (s.match(/\{(\w+)\}/g) ?? []).sort();

describe('i18n catalog', () => {
  it('ja と en のキー集合が一致する', () => {
    expect(Object.keys(CATALOG.en).sort()).toEqual(Object.keys(CATALOG.ja).sort());
  });

  it('全キーが両言語で非空文字列', () => {
    for (const locale of ['ja', 'en'] as const) {
      for (const [key, val] of Object.entries(CATALOG[locale])) {
        expect(val, `${locale}.${key}`).toBeTruthy();
      }
    }
  });

  it('プレースホルダ {x} は両言語で対応する', () => {
    for (const key of Object.keys(CATALOG.ja) as Array<keyof typeof CATALOG.ja>) {
      expect(placeholders(CATALOG.en[key]), key).toEqual(placeholders(CATALOG.ja[key]));
    }
  });
});
