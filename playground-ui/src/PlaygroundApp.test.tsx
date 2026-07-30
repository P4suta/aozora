import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { Mock } from 'vitest';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { PlaygroundApp } from './PlaygroundApp';
import { loadPreferences } from './storage';
import type {
  EditorController,
  PlaygroundAdapter,
  PlaygroundAnalysis,
  PreviewController,
} from './types';

function result(
  source: string,
  diagnostics: PlaygroundAnalysis['diagnostics'] = [],
): PlaygroundAnalysis {
  return {
    html: `<p>${source}</p>`,
    diagnostics,
    outline: [
      {
        level: 1,
        text: 'Welcome',
        range: { start: 0, end: source.length },
      },
    ],
  };
}

interface FakeHarness {
  readonly analyze: Mock;
  readonly adapter: PlaygroundAdapter;
  readonly destroyEditor: Mock;
  readonly focus: Mock;
  readonly initialize: Mock;
  readonly revealRange: Mock;
  readonly runCommand: Mock;
  readonly setLocale: Mock;
  readonly setSetting: Mock;
}

function fakeHarness(options?: {
  readonly analyze?: PlaygroundAdapter['analyze'];
  readonly initialize?: () => Promise<void>;
}): FakeHarness {
  const destroyEditor = vi.fn();
  const focus = vi.fn();
  const initialize = vi.fn(options?.initialize ?? (async () => {}));
  const revealRange = vi.fn();
  const runCommand = vi.fn(() => true);
  const setLocale = vi.fn();
  const setSetting = vi.fn();
  const analyze = vi.fn(
    options?.analyze ??
      (async (source: string) => {
      const diagnostics = source.includes('warning')
        ? [
            {
              severity: 'warning' as const,
              message: { ja: '新しい警告', en: 'New warning' },
              range: { start: 0, end: 7 },
              code: 'fake::warning',
            },
          ]
        : [];
        return result(source, diagnostics);
      }),
  );

  return {
    analyze,
    destroyEditor,
    focus,
    initialize,
    revealRange,
    runCommand,
    setLocale,
    setSetting,
    adapter: {
      product: {
        id: 'fake',
        name: 'Fake Writer',
        shortName: 'fake',
        description: { ja: '偽エンジン', en: 'Fake engine' },
        repositoryUrl: 'https://example.test/repository',
        engineVersion: '1.0.0',
      },
      samples: [
        {
          id: 'welcome',
          title: { ja: 'ようこそ', en: 'Welcome' },
          source: '# Welcome',
        },
        {
          id: 'second',
          title: { ja: '二つ目', en: 'Second' },
          source: '# Second',
        },
      ],
      guide: {
        title: { ja: 'ガイド', en: 'Guide' },
        introduction: { ja: '説明', en: 'Introduction' },
        sections: [
          {
            id: 'reference',
            title: { ja: '参照', en: 'Reference' },
            body: { ja: '本文', en: 'Body' },
            href: 'https://example.test/guide',
          },
        ],
      },
      commands: [
        {
          id: 'wrap',
          label: { ja: '囲む', en: 'Wrap' },
          shortcut: 'Ctrl+W',
        },
      ],
      settings: [
        {
          id: 'assist',
          label: { ja: '入力支援', en: 'Input assistance' },
          description: { ja: '入力を支援します。', en: 'Assists editing.' },
          defaultValue: true,
        },
      ],
      setLocale,
      initialize,
      analyze,
      createEditor(parent, initialValue, onChange): EditorController {
        const textarea = document.createElement('textarea');
        textarea.setAttribute('aria-label', 'Fake editor');
        textarea.value = initialValue;
        textarea.addEventListener('input', () => onChange(textarea.value));
        parent.append(textarea);
        return {
          setValue(value: string) {
            textarea.value = value;
          },
          focus,
          revealRange,
          runCommand,
          setSetting,
          destroy() {
            destroyEditor();
            textarea.remove();
          },
        };
      },
      createPreview(parent): PreviewController {
        return {
          update(html) {
            parent.innerHTML = html;
          },
          destroy() {
            parent.replaceChildren();
          },
        };
      },
    },
  };
}

function selectNativeOption(label: string): void {
  const option = Array.from(document.querySelectorAll('option')).find(
    (candidate) => candidate.textContent === label,
  );
  if (!(option instanceof HTMLOptionElement)) {
    throw new Error(`Option not found: ${label}`);
  }
  const select = option.closest('select');
  if (!(select instanceof HTMLSelectElement)) {
    throw new Error(`Select not found for option: ${label}`);
  }
  fireEvent.change(select, { target: { value: option.value } });
}

describe('shared PlaygroundApp', () => {
  beforeEach(() => {
    localStorage.clear();
    history.replaceState(null, '', '/');
    vi.stubGlobal('matchMedia', (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener() {},
      removeEventListener() {},
      addListener() {},
      removeListener() {},
      dispatchEvent: () => false,
    }));
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('runs through the adapter and executes a command from the keyboard palette', async () => {
    const harness = fakeHarness();
    render(<PlaygroundApp adapter={harness.adapter} />);
    expect(
      screen.getByRole('heading', { name: 'Fake Writer' }),
    ).toBeInTheDocument();
    await screen.findByLabelText('Fake editor');
    await waitFor(() =>
      expect(screen.getByText('# Welcome')).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole('radio', { name: 'Preview only' }));
    expect(screen.queryByLabelText('Fake editor')).not.toBeInTheDocument();

    fireEvent.keyDown(window, {
      key: 'P',
      code: 'KeyP',
      ctrlKey: true,
      shiftKey: true,
    });
    const palette = await screen.findByRole('dialog', {
      name: 'Command palette',
    });
    expect(palette).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Wrap/ }));
    await screen.findByLabelText('Fake editor');
    await waitFor(() => {
      expect(harness.runCommand).toHaveBeenCalledWith('wrap');
      expect(harness.focus).toHaveBeenCalled();
    });
  });

  it('shows a retryable Spectrum alert and recovers after initialization fails', async () => {
    const initialize = vi
      .fn<() => Promise<void>>()
      .mockRejectedValueOnce(new Error('boom'))
      .mockResolvedValueOnce();
    const harness = fakeHarness({ initialize });
    render(<PlaygroundApp adapter={harness.adapter} />);

    expect(
      await screen.findByText('WebAssembly failed to initialize.'),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await screen.findByLabelText('Fake editor');
    expect(initialize).toHaveBeenCalledTimes(2);
  });

  it('does not allow an older asynchronous analysis to overwrite a newer edit', async () => {
    const pending: Array<{
      source: string;
      resolve: (analysis: PlaygroundAnalysis) => void;
    }> = [];
    const analyze: PlaygroundAdapter['analyze'] = (source) =>
      new Promise((resolve) => pending.push({ source, resolve }));
    const harness = fakeHarness({ analyze });
    render(<PlaygroundApp adapter={harness.adapter} />);

    const editor = await screen.findByLabelText('Fake editor');
    await waitFor(() => expect(pending).toHaveLength(1));
    fireEvent.input(editor, { target: { value: 'newer' } });
    await waitFor(() => expect(pending).toHaveLength(2));

    pending[1]!.resolve(result('newer'));
    await screen.findByText('newer');
    pending[0]!.resolve(result('older'));
    await Promise.resolve();
    expect(screen.getByText('newer')).toBeInTheDocument();
    expect(screen.queryByText('older')).not.toBeInTheDocument();
  });

  it('expands only a newly introduced warning and jumps to its UTF-16 range', async () => {
    const harness = fakeHarness();
    render(<PlaygroundApp adapter={harness.adapter} />);

    const editor = await screen.findByLabelText('Fake editor');
    await waitFor(() =>
      expect(screen.getByText('# Welcome')).toBeInTheDocument(),
    );
    fireEvent.input(editor, { target: { value: 'warning' } });
    const warning = await screen.findByRole('button', {
      name: /New warning/,
    });
    fireEvent.click(screen.getByRole('radio', { name: 'Preview only' }));
    expect(screen.queryByLabelText('Fake editor')).not.toBeInTheDocument();
    fireEvent.click(warning);
    await screen.findByLabelText('Fake editor');
    await waitFor(() => {
      expect(harness.revealRange).toHaveBeenCalledWith({ start: 0, end: 7 });
      expect(harness.focus).toHaveBeenCalled();
    });
  });

  it('labels every diagnostic severity for sighted and assistive users', async () => {
    const harness = fakeHarness({
      analyze: async (source) =>
        result(source, [
          {
            severity: 'error',
            message: { ja: '壊れています', en: 'Broken input' },
            range: { start: 0, end: 1 },
          },
          {
            severity: 'warning',
            message: { ja: '確認してください', en: 'Check input' },
            range: { start: 1, end: 2 },
          },
          {
            severity: 'info',
            message: { ja: '参考情報', en: 'Helpful context' },
            range: { start: 2, end: 3 },
          },
        ]),
    });
    render(<PlaygroundApp adapter={harness.adapter} />);
    fireEvent.click(
      await screen.findByRole('button', { name: 'Diagnostics (3)' }),
    );

    expect(
      await screen.findByRole('button', { name: 'Error: Broken input' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Warning: Check input' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Information: Helpful context' }),
    ).toBeInTheDocument();
  });

  it('opens the outline, jumps to a heading, and destroys editor ownership', async () => {
    const harness = fakeHarness();
    const view = render(<PlaygroundApp adapter={harness.adapter} />);
    await screen.findByLabelText('Fake editor');
    await waitFor(() =>
      expect(screen.getByText('# Welcome')).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByRole('button', { name: 'Outline' }));
    fireEvent.click(screen.getByRole('button', { name: 'Welcome' }));
    expect(harness.revealRange).toHaveBeenCalledWith({
      start: 0,
      end: '# Welcome'.length,
    });
    view.unmount();
    expect(harness.destroyEditor).toHaveBeenCalledTimes(1);
  });

  it('loads samples, opens help surfaces, and shares only on request', async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(async () => {});
    vi.stubGlobal('navigator', {
      language: 'en-US',
      clipboard: { writeText },
    });
    const harness = fakeHarness();
    render(<PlaygroundApp adapter={harness.adapter} />);
    await screen.findByLabelText('Fake editor');

    selectNativeOption('Second');
    await screen.findByText('# Second');
    expect(location.hash).toBe('');

    await user.click(screen.getByRole('button', { name: 'Guide' }));
    expect(
      await screen.findByRole('dialog', { name: 'Guide' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('link', { name: 'https://example.test/guide' }),
    ).toHaveAttribute('href', 'https://example.test/guide');
    await user.keyboard('{Escape}');
    await waitFor(() =>
      expect(screen.queryByRole('dialog', { name: 'Guide' })).toBeNull(),
    );

    await user.click(
      screen.getByRole('button', { name: 'About this playground' }),
    );
    expect(
      await screen.findByRole('dialog', { name: 'About this playground' }),
    ).toHaveTextContent('Engine: 1.0.0');
    await user.keyboard('{Escape}');
    await waitFor(() =>
      expect(
        screen.queryByRole('dialog', { name: 'About this playground' }),
      ).toBeNull(),
    );

    await user.click(screen.getByRole('button', { name: 'Share' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledOnce());
    expect(location.hash).toMatch(/^#src=/);
  });

  it('updates theme, locale, title, and product editor settings', async () => {
    const user = userEvent.setup();
    const harness = fakeHarness();
    render(<PlaygroundApp adapter={harness.adapter} />);
    await screen.findByLabelText('Fake editor');
    expect(document.title).toBe('Fake Writer — Fake engine');

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    selectNativeOption('Dark');
    await waitFor(() =>
      expect(document.documentElement.dataset.colorScheme).toBe('dark'),
    );

    selectNativeOption('Japanese');
    await waitFor(() => {
      expect(document.documentElement.lang).toBe('ja');
      expect(document.title).toBe('Fake Writer — 偽エンジン');
      expect(harness.setLocale).toHaveBeenLastCalledWith('ja');
    });

    await user.click(screen.getByRole('switch', { name: '入力支援' }));
    expect(harness.setSetting).toHaveBeenLastCalledWith('assist', false);
    expect(loadPreferences()).toMatchObject({
      colorScheme: 'dark',
      locale: 'ja',
    });
  });

  it('closes a mobile outline before restoring editor focus', async () => {
    const user = userEvent.setup();
    vi.stubGlobal('matchMedia', (query: string) => ({
      matches: query.includes('max-width'),
      media: query,
      onchange: null,
      addEventListener() {},
      removeEventListener() {},
      addListener() {},
      removeListener() {},
      dispatchEvent: () => false,
    }));
    const harness = fakeHarness();
    render(<PlaygroundApp adapter={harness.adapter} />);
    await screen.findByLabelText('Fake editor');
    await waitFor(() => expect(harness.analyze).toHaveBeenCalledOnce());

    await user.click(screen.getByRole('button', { name: 'Outline' }));
    const dialog = await screen.findByRole('dialog', { name: 'Outline' });
    await user.click(within(dialog).getByRole('button', { name: 'Welcome' }));

    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: 'Outline' })).toBeNull();
      expect(harness.revealRange).toHaveBeenCalledWith({
        start: 0,
        end: '# Welcome'.length,
      });
      expect(harness.focus).toHaveBeenCalled();
    });
  });

  it('keeps the last successful preview visible when a later analysis fails', async () => {
    const analyze = vi.fn(async (source: string) => {
      if (source === 'broken') throw new Error('render failed');
      return result(source);
    });
    const harness = fakeHarness({ analyze });
    render(<PlaygroundApp adapter={harness.adapter} />);
    const editor = await screen.findByLabelText('Fake editor');
    await screen.findByText('# Welcome');

    fireEvent.input(editor, { target: { value: 'broken' } });
    expect(
      await screen.findByText('The preview could not be generated.'),
    ).toBeInTheDocument();
    expect(screen.getByText('# Welcome')).toBeInTheDocument();
  });
});
