import { describe, expect, it } from 'vitest';
import { DEFAULT_SAMPLE_ID, SAMPLES } from '../samples';

describe('samples.ts', () => {
  it('全サンプルの id が一意', () => {
    const ids = SAMPLES.map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('全サンプルが非空の id / title / text / source を持つ', () => {
    for (const s of SAMPLES) {
      expect(s.id).toBeTruthy();
      expect(s.title.length).toBeGreaterThan(0);
      expect(s.text.length).toBeGreaterThan(0);
      // Every sample is a cited public-domain excerpt, not an invented string.
      expect(s.source.length).toBeGreaterThan(0);
    }
  });

  it('DEFAULT_SAMPLE_ID は実在するサンプルを指す', () => {
    expect(SAMPLES.some((s) => s.id === DEFAULT_SAMPLE_ID)).toBe(true);
  });
});
