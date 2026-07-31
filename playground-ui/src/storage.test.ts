import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  defaultPreferences,
  loadDraft,
  loadPreferences,
  loadSettingValues,
  saveDraft,
  savePreferences,
  saveSettingValues,
} from './storage';

describe('playground persistence', () => {
  beforeEach(() => {
    localStorage.clear();
    history.replaceState(null, '', '/');
    vi.stubGlobal('navigator', { language: 'en-US' });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('uses authoring defaults and persists shared preferences', () => {
    expect(defaultPreferences()).toMatchObject({
      locale: 'en',
      layout: 'split',
      writingDirection: 'horizontal',
      outlineOpen: false,
    });
    const preferences = {
      ...defaultPreferences(),
      colorScheme: 'dark' as const,
      writingDirection: 'vertical' as const,
    };
    expect(savePreferences(preferences)).toBe(true);
    expect(loadPreferences()).toEqual(preferences);
  });

  it('migrates old shared display keys once', () => {
    localStorage.setItem('aozora-md-playground:color-scheme', 'light');
    localStorage.setItem('aozora-md-playground:theme-mode', 'vertical');
    localStorage.setItem('aozora-playground:locale', 'ja');
    const first = loadPreferences();
    expect(first).toMatchObject({
      colorScheme: 'light',
      locale: 'ja',
      writingDirection: 'vertical',
    });
    expect(loadPreferences()).toEqual(first);
    expect(localStorage.getItem('aozora-playground:migrated:v2')).toBe('1');
  });

  it('recovers legacy values left behind by an incomplete migration', () => {
    localStorage.setItem('aozora-playground:migrated:v2', '1');
    localStorage.setItem('aozora-playground:theme', 'dark');
    localStorage.setItem('aozora-playground:locale', 'ja');

    expect(loadPreferences()).toMatchObject({
      colorScheme: 'dark',
      locale: 'ja',
    });
    expect(localStorage.getItem('aozora-playground:preferences:v2')).not.toBe(
      null,
    );
  });

  it('prefers current values to legacy values and ignores malformed storage', () => {
    localStorage.setItem('aozora-md-playground:color-scheme', 'light');
    localStorage.setItem(
      'aozora-playground:preferences:v2',
      JSON.stringify({
        colorScheme: 'dark',
        locale: 'broken',
        layout: 'broken',
        writingDirection: 'broken',
        outlineOpen: 'broken',
      }),
    );
    expect(loadPreferences()).toMatchObject({
      colorScheme: 'dark',
      locale: 'en',
      layout: 'split',
      writingDirection: 'horizontal',
      outlineOpen: false,
    });

    localStorage.setItem('aozora-playground:preferences:v2', '{');
    expect(loadPreferences()).toMatchObject({
      colorScheme: 'auto',
      locale: 'en',
    });
  });

  it('defaults non-English browser locales to Japanese', () => {
    vi.stubGlobal('navigator', { language: 'fr-FR' });
    expect(defaultPreferences().locale).toBe('ja');
  });

  it('keeps the legacy language query as the highest locale preference', () => {
    localStorage.setItem(
      'aozora-playground:preferences:v2',
      JSON.stringify({ locale: 'en' }),
    );
    history.replaceState(null, '', '/?lang=ja');
    expect(loadPreferences().locale).toBe('ja');
  });

  it('keeps drafts and editor settings product scoped', () => {
    expect(saveDraft('afm', 'draft')).toBe(true);
    expect(loadDraft('afm')).toBe('draft');
    expect(loadDraft('aozora')).toBeNull();

    const settings = [
      {
        id: 'assist',
        label: { ja: '支援', en: 'Assist' },
        description: { ja: '説明', en: 'Description' },
        defaultValue: true,
      },
    ];
    expect(loadSettingValues('afm', settings)).toEqual({ assist: true });
    expect(saveSettingValues('afm', { assist: false })).toBe(true);
    expect(loadSettingValues('afm', settings)).toEqual({ assist: false });
  });

  it('degrades safely when browser storage throws', () => {
    vi.stubGlobal('localStorage', {
      getItem: () => {
        throw new DOMException('denied');
      },
      setItem: () => {
        throw new DOMException('denied');
      },
    });
    expect(loadDraft('afm')).toBeNull();
    expect(saveDraft('afm', 'draft')).toBe(false);
    expect(savePreferences(defaultPreferences())).toBe(false);
  });
});
