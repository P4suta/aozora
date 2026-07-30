import { describe, expect, it } from 'vitest';

import { GALLERY_FIXTURES } from './gallery-fixtures';

describe('notation gallery fixtures', () => {
  it('covers every visible representative family with unique metadata', () => {
    expect(GALLERY_FIXTURES.map(({ family }) => family)).toEqual([
      'ruby',
      'bouten',
      'tcy',
      'kaeriten',
      'gaiji',
      'angle-quote',
      'warichu',
    ]);
    expect(new Set(GALLERY_FIXTURES.map(({ family }) => family)).size).toBe(
      GALLERY_FIXTURES.length,
    );
    for (const fixture of GALLERY_FIXTURES) {
      expect(fixture.label.ja).not.toBe('');
      expect(fixture.label.en).not.toBe('');
      expect(fixture.source).not.toBe('');
    }
  });
});
