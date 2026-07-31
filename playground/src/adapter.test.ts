import { playgroundAdapterContract } from '@aozora/playground-ui/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { aozoraPlaygroundAdapter } from './adapter';
import {
  analyze,
  deriveOutline,
  initializeEngine,
  normalizeDiagnostics,
} from './adapter-engine';
import { Document } from './wasm-loader';

playgroundAdapterContract('Aozora Notation', aozoraPlaygroundAdapter);

describe('Aozora PlaygroundAdapter', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('normalizes byte diagnostics to localized UTF-16 ranges', () => {
    expect(
      normalizeDiagnostics('😀》', [
        {
          kind: 'unmatched_close',
          severity: 'error',
          source: 'source',
          span: { start: 4, end: 7 },
        },
      ]),
    ).toEqual([
      {
        severity: 'error',
        message: {
          ja: '閉じ括弧に対応する開き括弧がありません。',
          en: 'A close bracket has no matching open bracket.',
        },
        range: { start: 2, end: 3 },
        code: 'aozora::lex::unmatched_close',
      },
    ]);
  });

  it('normalizes notes, lint codes, unknown kinds, and invalid spans', () => {
    expect(
      normalizeDiagnostics('abc', [
        {
          kind: 'non_canonical_directive',
          severity: 'note',
          source: 'source',
          span: { start: 99, end: 1 },
        },
        {
          kind: 'future_diagnostic',
          severity: 'warning',
          source: 'internal',
          span: { start: 1, end: 2 },
        },
      ]),
    ).toEqual([
      {
        severity: 'info',
        message: {
          ja: '注記が正規の綴りではありません。',
          en: 'The annotation uses a non-canonical spelling.',
        },
        range: { start: 3, end: 3 },
        code: 'aozora::lint::non_canonical_directive',
      },
      {
        severity: 'warning',
        message: {
          ja: '診断: future diagnostic',
          en: 'future diagnostic',
        },
        range: { start: 1, end: 2 },
        code: 'aozora::lex::future_diagnostic',
      },
    ]);
  });

  it('derives forward and container headings in source order', () => {
    const source =
      '序章\n［＃「序章」は中見出し］\n［＃ここから大見出し］\n本章\n［＃ここで大見出し終わり］';
    const bytes = new TextEncoder();
    const byteOffset = (text: string) =>
      bytes.encode(source.slice(0, source.indexOf(text))).length;
    const hint = '［＃「序章」は中見出し］';
    const open = '［＃ここから大見出し］';
    const close = '［＃ここで大見出し終わり］';

    expect(
      deriveOutline(
        source,
        [
          {
            kind: 'headingHint',
            span: {
              start: byteOffset(hint),
              end: byteOffset(hint) + bytes.encode(hint).length,
            },
          },
        ],
        [
          {
            kind: 'heading',
            open: {
              start: byteOffset(open),
              end: byteOffset(open) + bytes.encode(open).length,
            },
            close: {
              start: byteOffset(close),
              end: byteOffset(close) + bytes.encode(close).length,
            },
          },
        ],
      ),
    ).toEqual([
      { level: 2, text: '序章', range: { start: 0, end: 2 } },
      {
        level: 1,
        text: '本章',
        range: {
          start: source.indexOf('本章'),
          end: source.indexOf('本章') + 2,
        },
      },
    ]);
  });

  it('derives every node heading form and ignores malformed candidates', () => {
    const source = [
      '平文見出し',
      '小見出し本文［＃小見出し］',
      'ヒント対象',
      '［＃「ヒント対象」は中見出し］',
      '［＃対象なし］',
      '［＃「不在」は大見出し］',
      '［＃ここから小見出し］',
      'コンテナ本文',
      '［＃ここで小見出し終わり］',
      '［＃ここから大見出し］',
      '末尾本文',
    ].join('\n');
    const encoder = new TextEncoder();
    const span = (value: string, from = 0) => {
      const start = source.indexOf(value, from);
      return {
        start: encoder.encode(source.slice(0, start)).length,
        end:
          encoder.encode(source.slice(0, start)).length +
          encoder.encode(value).length,
      };
    };
    const plain = span('平文見出し');
    const annotated = span('小見出し本文［＃小見出し］');
    const validHint = span('［＃「ヒント対象」は中見出し］');
    const noTargetHint = span('［＃対象なし］');
    const missingHint = span('［＃「不在」は大見出し］');
    const smallOpen = span('［＃ここから小見出し］');
    const smallClose = span('［＃ここで小見出し終わり］');
    const finalOpen = span('［＃ここから大見出し］');

    expect(
      deriveOutline(
        source,
        [
          { kind: 'heading', span: plain },
          { kind: 'heading', span: annotated },
          { kind: 'heading', span: span('［＃対象なし］') },
          { kind: 'headingHint', span: validHint },
          { kind: 'headingHint', span: noTargetHint },
          { kind: 'headingHint', span: missingHint },
          { kind: 'containerOpen', span: smallOpen },
          { kind: 'containerOpen', span: smallClose },
          { kind: 'containerOpen', span: finalOpen },
          { kind: 'ruby', span: plain },
        ],
        [
          {
            kind: 'heading',
            open: smallOpen,
            close: smallClose,
          },
          {
            kind: 'indent',
            open: smallOpen,
            close: smallClose,
          },
          {
            kind: 'heading',
            open: span('［＃対象なし］'),
            close: span('［＃対象なし］'),
          },
        ],
      ),
    ).toEqual([
      {
        level: 1,
        text: '平文見出し',
        range: {
          start: source.indexOf('平文見出し'),
          end: source.indexOf('平文見出し') + '平文見出し'.length,
        },
      },
      {
        level: 3,
        text: '小見出し本文',
        range: {
          start: source.indexOf('小見出し本文'),
          end: source.indexOf('小見出し本文') + '小見出し本文'.length,
        },
      },
      {
        level: 2,
        text: 'ヒント対象',
        range: {
          start: source.indexOf('ヒント対象'),
          end: source.indexOf('ヒント対象') + 'ヒント対象'.length,
        },
      },
      {
        level: 3,
        text: 'コンテナ本文',
        range: {
          start: source.indexOf('コンテナ本文'),
          end: source.indexOf('コンテナ本文') + 'コンテナ本文'.length,
        },
      },
      {
        level: 1,
        text: '末尾本文',
        range: {
          start: source.indexOf('末尾本文'),
          end: source.length,
        },
      },
    ]);
  });

  it('initializes and analyzes through the WASM document lifecycle', async () => {
    const free = vi.spyOn(Document.prototype, 'free');

    await initializeEngine();
    await expect(
      analyze('😀》', {
        revision: 1,
        signal: new AbortController().signal,
      }),
    ).resolves.toMatchObject({
      html: '<p>😀》</p>',
      diagnostics: [
        {
          severity: 'error',
          range: { start: 2, end: 3 },
        },
      ],
      outline: [],
    });
    expect(free).toHaveBeenCalledOnce();
  });

  it('rejects before and after analysis when aborted and always frees', async () => {
    const free = vi.spyOn(Document.prototype, 'free');
    const alreadyAborted = new AbortController();
    alreadyAborted.abort();

    await expect(
      analyze('before', {
        revision: 1,
        signal: alreadyAborted.signal,
      }),
    ).rejects.toMatchObject({ name: 'AbortError' });
    expect(free).not.toHaveBeenCalled();

    let reads = 0;
    const signal = {
      get aborted() {
        reads += 1;
        return reads > 1;
      },
    } as AbortSignal;
    await expect(
      analyze('after', { revision: 2, signal }),
    ).rejects.toMatchObject({ name: 'AbortError' });
    expect(free).toHaveBeenCalledOnce();
  });

  it('frees the WASM document when rendering throws', async () => {
    const free = vi.spyOn(Document.prototype, 'free');
    vi.spyOn(Document.prototype, 'toHtml').mockImplementationOnce(() => {
      throw new Error('render failed');
    });

    await expect(
      analyze('source', {
        revision: 1,
        signal: new AbortController().signal,
      }),
    ).rejects.toThrow('render failed');
    expect(free).toHaveBeenCalledOnce();
  });

  it('owns the preview HTML boundary and writing direction class', () => {
    const host = document.createElement('div');
    const preview = aozoraPlaygroundAdapter.createPreview(host);

    preview.update('<ruby>青空<rt>あおぞら</rt></ruby>', 'vertical');
    const ruby = host.querySelector('.aozora-notation ruby');
    expect(ruby?.innerHTML).toContain('青空');
    expect(host.querySelector('.aozora-vertical')).not.toBeNull();

    preview.update('<ruby>青空<rt>あおぞら</rt></ruby>', 'horizontal');
    expect(host.querySelector('.aozora-notation ruby')).toBe(ruby);
    expect(host.querySelector('.aozora-vertical')).toBeNull();

    preview.destroy();
    expect(host.childElementCount).toBe(0);
  });

  it('updates the active editor locale without replacing its view', async () => {
    aozoraPlaygroundAdapter.setLocale?.('en');
    const host = document.createElement('div');
    const editor = await aozoraPlaygroundAdapter.createEditor(host, '', () => {
      throw new Error('locale refresh must not edit the document');
    });
    const content = host.querySelector('.cm-content');
    expect(content?.getAttribute('aria-label')).toBe('Aozora notation source');
    expect(host.querySelector('.cm-placeholder')?.textContent).toBe(
      'Type Aozora notation…',
    );

    aozoraPlaygroundAdapter.setLocale?.('ja');
    expect(host.querySelector('.cm-content')).toBe(content);
    expect(content?.getAttribute('aria-label')).toBe('入力（青空文庫記法）');
    expect(host.querySelector('.cm-placeholder')?.textContent).toBe(
      '青空文庫記法を入力…',
    );

    editor.destroy();
    aozoraPlaygroundAdapter.setLocale?.('en');
  });
});
