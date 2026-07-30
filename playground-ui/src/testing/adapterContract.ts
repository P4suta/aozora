import { describe, expect, it } from 'vitest';

import type { PlaygroundAdapter } from '../types';

function expectLocalized(value: { ja: string; en: string }): void {
  expect(value.ja.trim()).not.toBe('');
  expect(value.en.trim()).not.toBe('');
}

function expectUnique(values: readonly string[]): void {
  expect(new Set(values).size).toBe(values.length);
}

export function playgroundAdapterContract(
  name: string,
  adapter: PlaygroundAdapter,
): void {
  describe(`${name} PlaygroundAdapter contract`, () => {
    it('provides complete, stable, localized authoring metadata', () => {
      expect(adapter.product.id).toMatch(/^[a-z0-9-]+$/);
      expect(adapter.product.name.trim()).not.toBe('');
      expect(adapter.product.shortName.trim()).not.toBe('');
      expect(adapter.product.engineVersion.trim()).not.toBe('');
      expect(() => new URL(adapter.product.repositoryUrl)).not.toThrow();
      expectLocalized(adapter.product.description);

      expect(adapter.samples.length).toBeGreaterThan(0);
      expectUnique(adapter.samples.map(({ id }) => id));
      for (const sample of adapter.samples) {
        expect(sample.id.trim()).not.toBe('');
        expect(sample.source.trim()).not.toBe('');
        expectLocalized(sample.title);
      }

      expectLocalized(adapter.guide.title);
      expectLocalized(adapter.guide.introduction);
      expectUnique(adapter.guide.sections.map(({ id }) => id));
      for (const section of adapter.guide.sections) {
        expectLocalized(section.title);
        expectLocalized(section.body);
        const href = section.href;
        if (href) expect(() => new URL(href)).not.toThrow();
      }

      expectUnique(adapter.commands.map(({ id }) => id));
      for (const command of adapter.commands) {
        expectLocalized(command.label);
      }
      expectUnique(adapter.settings.map(({ id }) => id));
      for (const setting of adapter.settings) {
        expectLocalized(setting.label);
        expectLocalized(setting.description);
        expect(typeof setting.defaultValue).toBe('boolean');
      }
      adapter.setLocale?.('ja');
      adapter.setLocale?.('en');
    });

    it('initializes idempotently and returns normalized analysis', async () => {
      await adapter.initialize();
      await adapter.initialize();
      const source = adapter.samples[0]!.source;
      const analysis = await adapter.analyze(source, {
        revision: 1,
        signal: new AbortController().signal,
      });

      expect(typeof analysis.html).toBe('string');
      expect(analysis.html.length).toBeGreaterThan(0);
      for (const diagnostic of analysis.diagnostics) {
        expect(['info', 'warning', 'error']).toContain(diagnostic.severity);
        expectLocalized(diagnostic.message);
        expect(diagnostic.range.start).toBeGreaterThanOrEqual(0);
        expect(diagnostic.range.end).toBeGreaterThanOrEqual(
          diagnostic.range.start,
        );
        expect(diagnostic.range.end).toBeLessThanOrEqual(source.length);
      }
      for (const entry of analysis.outline) {
        expect(entry.level).toBeGreaterThanOrEqual(1);
        expect(entry.level).toBeLessThanOrEqual(6);
        expect(entry.text.trim()).not.toBe('');
        if (entry.range) {
          expect(entry.range.start).toBeGreaterThanOrEqual(0);
          expect(entry.range.end).toBeLessThanOrEqual(source.length);
        }
      }
    });

    it('honors an already-aborted analysis request', async () => {
      await adapter.initialize();
      const abort = new AbortController();
      abort.abort();
      await expect(
        adapter.analyze(adapter.samples[0]!.source, {
          revision: 2,
          signal: abort.signal,
        }),
      ).rejects.toMatchObject({ name: 'AbortError' });
    });

    it('owns editor and preview lifecycles through controllers', async () => {
      await adapter.initialize();
      const source = adapter.samples[0]!.source;
      const editorHost = document.createElement('div');
      const previewHost = document.createElement('div');
      document.body.append(editorHost, previewHost);
      const editor = await adapter.createEditor(editorHost, source, () => {});
      const preview = adapter.createPreview(previewHost);
      const analysis = await adapter.analyze(source, {
        revision: 3,
        signal: new AbortController().signal,
      });

      editor.setValue(source);
      editor.revealRange({ start: 0, end: Math.min(1, source.length) });
      editor.focus();
      for (const setting of adapter.settings) {
        editor.setSetting(setting.id, setting.defaultValue);
      }
      for (const command of adapter.commands) {
        expect(editor.runCommand(command.id)).toBe(true);
      }
      preview.update(analysis.html, 'horizontal');
      preview.update(analysis.html, 'vertical');
      expect(previewHost.childElementCount).toBeGreaterThan(0);

      editor.destroy();
      preview.destroy();
      expect(editorHost.childElementCount).toBe(0);
      expect(previewHost.childElementCount).toBe(0);
      editorHost.remove();
      previewHost.remove();
    });
  });
}
