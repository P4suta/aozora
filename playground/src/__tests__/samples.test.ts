import { describe, expect, it } from 'vitest';
import { DEFAULT_SAMPLE_ID, SAMPLES } from '../samples';

describe('samples.ts', () => {
  it('uses a unique ID for every sample', () => {
    const ids = SAMPLES.map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('gives every sample a non-empty ID, title, text, and source', () => {
    for (const s of SAMPLES) {
      expect(s.id).toBeTruthy();
      expect(s.title.length).toBeGreaterThan(0);
      expect(s.text.length).toBeGreaterThan(0);
      // Every sample is a cited public-domain excerpt, not an invented string.
      expect(s.source.length).toBeGreaterThan(0);
    }
  });

  it('points DEFAULT_SAMPLE_ID to an existing sample', () => {
    expect(SAMPLES.some((s) => s.id === DEFAULT_SAMPLE_ID)).toBe(true);
  });
});
