import AxeBuilder from '@axe-core/playwright';
import { expect, type Page, test } from '@playwright/test';
import LZString from 'lz-string';

import { SAMPLES } from '../src/samples';

const clientErrors = new WeakMap<Page, string[]>();

test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  clientErrors.set(page, errors);
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
});

test.afterEach(async ({ page }) => {
  expect(clientErrors.get(page) ?? []).toEqual([]);
});

async function openPlayground(page: Page): Promise<void> {
  await page.goto('./');
  await expect(
    page.getByRole('heading', { name: 'Aozora Notation', exact: true }),
  ).toBeVisible();
  await expect(page.locator('.cm-editor')).toBeVisible();
  if ((page.viewportSize()?.width ?? 0) >= 768) {
    const preview = page.locator('.playground-preview-host');
    await expect(preview).toBeVisible();
    await expect(preview.locator('.aozora-notation')).not.toBeEmpty();
  }
}

async function replaceEditor(page: Page, source: string): Promise<void> {
  const editor = page.locator('.cm-content');
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  if (source === '') {
    await page.keyboard.press('Backspace');
  } else {
    await page.keyboard.insertText(source);
  }
}

async function expectNoAxeViolations(page: Page): Promise<void> {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(results.violations).toEqual([]);
}

async function chooseOverflowAction(
  page: Page,
  name: string | RegExp,
): Promise<void> {
  await page.getByRole('button', { name: 'More' }).click();
  await page.getByRole('menuitem', { name }).click();
}

function sampleText(id: string): string {
  const sample = SAMPLES.find((candidate) => candidate.id === id);
  if (!sample) throw new Error(`sample '${id}' not found`);
  return sample.text;
}

test.describe('desktop authoring workspace', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test.beforeEach(async ({ page }) => {
    await openPlayground(page);
  });

  test('renders the authoring frame and fixed pane scrolling', async ({
    page,
  }) => {
    await page.keyboard.press('Control+Shift+P');
    await expect(
      page.getByRole('dialog', { name: 'Command palette' }),
    ).toBeVisible();

    const dimensions = await page.evaluate(() => ({
      body: document.body.scrollHeight,
      document: document.documentElement.scrollHeight,
      viewport: window.innerHeight,
    }));
    expect(dimensions.body).toBeLessThanOrEqual(dimensions.viewport);
    expect(dimensions.document).toBeLessThanOrEqual(dimensions.viewport);
  });

  test('switches all layouts and jumps through the persistent outline', async ({
    page,
  }) => {
    await page.getByRole('radio', { name: 'Editor only' }).click();
    await expect(page.getByRole('region', { name: 'Preview' })).toBeHidden();
    await page.getByRole('radio', { name: 'Preview only' }).click();
    await expect(page.getByRole('region', { name: 'Editor' })).toBeHidden();
    await page.getByRole('radio', { name: 'Split' }).click();

    await replaceEditor(page, '第一章\n［＃「第一章」は大見出し］');
    await page.getByRole('button', { name: 'Outline' }).click();
    const outline = page.getByRole('complementary', { name: 'Outline' });
    await expect(outline.getByRole('button', { name: '第一章' })).toBeVisible();
    await outline.getByRole('button', { name: '第一章' }).click();
    await expect(page.locator('.cm-content')).toBeFocused();
  });

  test('runs notation commands and updates the real WASM preview', async ({
    page,
  }) => {
    await replaceEditor(page, '漢字');
    await page.keyboard.press('ControlOrMeta+A');
    await page.keyboard.press('Control+Shift+P');
    await page.getByRole('button', { name: /Ruby/ }).click();
    await expect(page.locator('.cm-content')).toBeFocused();
    await page.keyboard.insertText('かんじ');

    await expect(page.locator('.cm-content')).toContainText('｜漢字《かんじ》');
    await expect(page.locator('.playground-preview-host ruby')).toContainText(
      '漢字',
    );
  });

  test('accepts completion snippets and folds annotation containers from the keyboard', async ({
    page,
  }) => {
    await replaceEditor(page, '');
    await page.keyboard.insertText('｜');
    await page.keyboard.press('Control+Space');
    const rubyCompletion = page.getByRole('option', {
      name: /Ruby \(explicit\)/,
    });
    await expect(rubyCompletion).toBeVisible();
    await expect(rubyCompletion).toHaveAttribute('aria-selected', 'true');
    await page.waitForTimeout(100);
    await page.keyboard.press('Enter');
    expect(await page.evaluate(() => getSelection()?.toString())).toBe('base');
    await page.keyboard.insertText('漢字');
    await page.keyboard.press('Tab');
    expect(await page.evaluate(() => getSelection()?.toString())).toBe(
      'reading',
    );
    await page.keyboard.insertText('かんじ');
    await page.keyboard.press('Tab');
    await expect(page.locator('.cm-content')).toContainText('｜漢字《かんじ》');
    await expect(
      page.locator('.playground-preview-host ruby rt'),
    ).toContainText('かんじ');

    await replaceEditor(page, sampleText('indent'));
    await page.keyboard.press('Control+Home');
    await page.keyboard.press('Control+Shift+BracketLeft');
    await expect(page.getByLabel('folded code')).toBeVisible();
    await expect(page.locator('.cm-content')).not.toContainText('花の頃');

    await page.keyboard.press('Control+Shift+BracketRight');
    await expect(page.getByLabel('folded code')).toHaveCount(0);
    await expect(page.locator('.cm-content')).toContainText('花の頃');
  });

  test('preserves editor history and folds while changing language', async ({
    page,
  }) => {
    await replaceEditor(page, sampleText('indent'));
    await page.keyboard.press('Control+End');
    await page.keyboard.insertText('\nlocale history marker');
    await page.keyboard.press('Control+Home');
    await page.keyboard.press('Control+Shift+BracketLeft');
    const originalEditor = await page.locator('.cm-editor').elementHandle();
    expect(originalEditor).not.toBeNull();
    await expect(page.getByLabel('folded code')).toBeVisible();

    await page.getByRole('button', { name: 'Settings' }).click();
    const settings = page.getByRole('dialog', { name: 'Settings' });
    await settings.getByRole('button', { name: /Language/ }).click();
    const japanese = page.getByRole('option', { name: 'Japanese' });
    await japanese.click();
    await expect(japanese).toBeHidden();
    const localizedSettings = page.getByRole('dialog', { name: '設定' });
    await expect(localizedSettings).toBeVisible();
    await localizedSettings.getByRole('button', { name: '閉じる' }).click();
    await expect(localizedSettings).toBeHidden();

    const localizedEditor = page.getByRole('textbox', {
      name: '入力（青空文庫記法）',
    });
    await expect(localizedEditor).toBeVisible();
    const updatedEditor = await page.locator('.cm-editor').elementHandle();
    expect(
      await originalEditor?.evaluate(
        (original, updated) => original.isSameNode(updated),
        updatedEditor,
      ),
    ).toBe(true);
    await expect(page.getByLabel('folded code')).toBeVisible();

    await localizedEditor.click();
    await page.keyboard.press('ControlOrMeta+Z');
    await expect(localizedEditor).not.toContainText('locale history marker');
    await page.keyboard.press('ControlOrMeta+Shift+Z');
    await expect(localizedEditor).toContainText('locale history marker');
  });

  test('keeps full-width conversion and gaiji inlay assistance configurable', async ({
    page,
  }) => {
    await replaceEditor(page, '');
    await page.locator('.cm-content').click();
    await page.keyboard.type('[');
    await expect(page.locator('.cm-content')).toContainText('［＃］');

    await replaceEditor(page, sampleText('gaiji'));
    await expect(page.locator('.cm-aozora-inlay')).toBeVisible();

    await page.getByRole('button', { name: 'Settings' }).click();
    const settings = page.getByRole('dialog', { name: 'Settings' });
    const gaijiHints = settings.getByRole('switch', {
      name: 'Gaiji inlay hints',
    });
    await gaijiHints.focus();
    await page.keyboard.press('Space');
    const conversion = settings.getByRole('switch', {
      name: 'Instant half-width → full-width',
    });
    await conversion.focus();
    await page.keyboard.press('Space');
    await page.keyboard.press('Escape');
    await expect(page.locator('.cm-aozora-inlay')).toHaveCount(0);

    await replaceEditor(page, '');
    await page.locator('.cm-content').click();
    await page.keyboard.type('[');
    await expect(page.locator('.cm-content')).toHaveText('[');
  });

  test('shows human diagnostics and selects astral-safe ranges from preview-only mode', async ({
    page,
  }) => {
    await replaceEditor(page, '😀》');
    await page.getByRole('radio', { name: 'Preview only' }).click();
    const disclosure = page.getByRole('button', { name: 'Diagnostics (1)' });
    await expect(disclosure).toHaveAttribute('aria-expanded', 'true');
    await page
      .getByRole('button', { name: /close bracket has no matching open/i })
      .click();
    await expect(page.getByRole('region', { name: 'Editor' })).toBeVisible();
    await expect(page.locator('.cm-content')).toBeFocused();
    expect(await page.evaluate(() => getSelection()?.toString())).toBe('》');
  });

  test('renders ruby, gaiji ruby, and tate-chu-yoko without source leakage', async ({
    page,
  }) => {
    for (const [source, reading] of [
      [sampleText('ruby'), 'ぞうしがや'],
      [sampleText('gaiji'), 'みは'],
      [
        '「川※［＃「くさかんむり／弓」、第3水準1-90-62］《せんきゅう》といふ」',
        'せんきゅう',
      ],
    ]) {
      await replaceEditor(page, source);
      await expect(
        page.locator('.playground-preview-host ruby rt'),
      ).toContainText(reading);
      await expect(page.locator('.playground-preview-host')).not.toContainText(
        '［＃',
      );
    }

    await replaceEditor(page, '明治33［＃「33」は縦中横］年');
    await page.getByRole('radio', { name: 'Vertical' }).click();
    const tcy = page.locator(
      '.playground-preview-host .aozora-combine-upright',
    );
    await expect(tcy).toContainText('33');
    expect(
      await tcy.evaluate(
        (element) => getComputedStyle(element).textCombineUpright,
      ),
    ).toBe('all');
  });

  test('changes writing direction without replacing the rendered document', async ({
    page,
  }) => {
    await replaceEditor(page, sampleText('ruby'));
    const ruby = page.locator('.playground-preview-host ruby').first();
    const originalRuby = await ruby.elementHandle();
    expect(originalRuby).not.toBeNull();

    await page.getByRole('radio', { name: 'Vertical' }).click();
    await expect(page.locator('.playground-preview-host')).toHaveAttribute(
      'data-writing-direction',
      'vertical',
    );
    const updatedRuby = await ruby.elementHandle();
    expect(
      await originalRuby?.evaluate(
        (original, updated) => original.isSameNode(updated),
        updatedRuby,
      ),
    ).toBe(true);
  });

  test('creates a reloadable hash only when Share is invoked', async ({
    context,
    page,
  }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await replaceEditor(page, '共有する原稿');
    await expect(page).not.toHaveURL(/#src=/);
    await page.getByRole('button', { name: 'Share' }).click();
    await expect(page).toHaveURL(/#src=/);
    expect(await page.evaluate(() => navigator.clipboard.readText())).toContain(
      '#src=',
    );
    await page.reload();
    await expect(page.locator('.cm-content')).toContainText('共有する原稿');
  });

  test('persists drafts, display preferences, samples, and editor assists', async ({
    page,
  }) => {
    await page.getByRole('button', { name: 'Sample' }).click();
    await page.getByRole('option', { name: /Explicit ruby/ }).click();
    await expect(page.locator('.playground-preview-host ruby')).toBeVisible();

    await page.getByRole('radio', { name: 'Preview only' }).click();
    await page.getByRole('button', { name: 'Outline' }).click();
    await page.getByRole('radio', { name: 'Vertical' }).click();
    await page.getByRole('button', { name: 'Settings' }).click();
    const settings = page.getByRole('dialog', { name: 'Settings' });
    await settings.getByRole('button', { name: /Theme/ }).click();
    await page.getByRole('option', { name: 'Dark' }).click();
    await settings.getByRole('button', { name: /Language/ }).click();
    const japanese = page.getByRole('option', { name: 'Japanese' });
    await japanese.click();
    await expect(japanese).toBeHidden();
    const conversion = page.getByRole('switch', {
      name: '半角→全角の即時変換',
    });
    await conversion.focus();
    await page.keyboard.press('Space');
    await page.keyboard.press('Escape');
    await page.waitForTimeout(350);
    await page.reload();

    await expect(page.locator('html')).toHaveAttribute('lang', 'ja');
    await expect(page.locator('html')).toHaveAttribute(
      'data-color-scheme',
      'dark',
    );
    await expect(page.getByRole('region', { name: 'エディタ' })).toBeHidden();
    await expect(page.locator('.playground-preview-host')).toHaveAttribute(
      'data-writing-direction',
      'vertical',
    );
    await page.getByRole('button', { name: '設定' }).click();
    await expect(
      page.getByRole('switch', { name: '半角→全角の即時変換' }),
    ).not.toBeChecked();
  });

  test('opens guide, settings, and About with focus restoration', async ({
    page,
  }) => {
    const guide = page.getByRole('button', { name: 'Guide' });
    await guide.click();
    const guideDialog = page.getByRole('dialog', {
      name: 'Aozora notation guide',
    });
    await expect(
      guideDialog.getByRole('link', {
        name: 'https://www.aozora.gr.jp/annotation/',
      }),
    ).toHaveAttribute('href', 'https://www.aozora.gr.jp/annotation/');
    await page.keyboard.press('Escape');
    await expect(guide).toBeFocused();

    const settings = page.getByRole('button', { name: 'Settings' });
    await settings.click();
    await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(settings).toBeFocused();

    const about = page.getByRole('button', {
      name: 'About this playground',
    });
    await about.click();
    const aboutDialog = page.getByRole('dialog', {
      name: 'About this playground',
    });
    await expect(aboutDialog).toContainText('Engine:');
    await expect(
      aboutDialog.getByRole('link', { name: 'Repository' }),
    ).toHaveAttribute('href', 'https://github.com/P4suta/aozora');
  });

  test('has no WCAG 2.2 AA violations in primary and modal states', async ({
    page,
  }) => {
    await expectNoAxeViolations(page);
    await page.getByRole('button', { name: 'Settings' }).click();
    await expectNoAxeViolations(page);
  });

  test('keeps the Japanese dark workspace and dialog accessible', async ({
    page,
  }) => {
    await page.evaluate(() => {
      localStorage.setItem(
        'aozora-playground:preferences:v2',
        JSON.stringify({
          colorScheme: 'dark',
          locale: 'ja',
          layout: 'split',
          writingDirection: 'horizontal',
          outlineOpen: false,
        }),
      );
    });
    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('lang', 'ja');
    await expect(page.locator('html')).toHaveAttribute(
      'data-color-scheme',
      'dark',
    );
    await expect(
      page.getByRole('textbox', { name: '入力（青空文庫記法）' }),
    ).toBeVisible();
    await expectNoAxeViolations(page);

    await page.getByRole('button', { name: '設定' }).click();
    await expect(page.getByRole('dialog', { name: '設定' })).toBeVisible();
    await expectNoAxeViolations(page);
    await page.keyboard.press('Escape');
    await page.evaluate(() => document.fonts.ready);
    await expect(page).toHaveScreenshot('desktop-ja-dark.png', {
      animations: 'disabled',
      maxDiffPixelRatio: 0.01,
    });
  });

  test('does not execute renderer-looking source as active markup', async ({
    page,
  }) => {
    await replaceEditor(
      page,
      '<img src=x onerror="globalThis.__aozoraInjected = true">',
    );
    await expect(page.locator('.playground-preview-host')).toContainText(
      '<img src=x',
    );
    expect(
      await page.evaluate(
        () =>
          (globalThis as typeof globalThis & { __aozoraInjected?: boolean })
            .__aozoraInjected,
      ),
    ).toBeUndefined();
  });

  test('ships a stable desktop visual state', async ({ page }) => {
    await page.evaluate(() => document.fonts.ready);
    await expect(page).toHaveScreenshot('desktop-default.png', {
      animations: 'disabled',
      maxDiffPixelRatio: 0.01,
    });
  });
});

test.describe('compact desktop action overflow', () => {
  test.use({ viewport: { width: 900, height: 800 } });

  test('reaches every primary header action through the overflow menu', async ({
    context,
    page,
  }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await openPlayground(page);
    await expect(page.getByRole('button', { name: 'More' })).toBeVisible();

    await chooseOverflowAction(page, /^Explicit ruby/);
    await expect(page.locator('.playground-preview-host ruby')).toBeVisible();

    await chooseOverflowAction(page, 'Preview only');
    await expect(page.getByRole('region', { name: 'Editor' })).toBeHidden();
    await chooseOverflowAction(page, 'Split');
    await expect(page.getByRole('region', { name: 'Editor' })).toBeVisible();

    await chooseOverflowAction(page, 'Notation commands');
    await expect(
      page.getByRole('dialog', { name: 'Command palette' }),
    ).toBeVisible();
    await page.keyboard.press('Escape');

    await chooseOverflowAction(page, 'Guide');
    await expect(
      page.getByRole('dialog', { name: 'Aozora notation guide' }),
    ).toBeVisible();
    await page.keyboard.press('Escape');

    await chooseOverflowAction(page, 'Share');
    await expect(page).toHaveURL(/#src=/);

    await chooseOverflowAction(page, 'Settings');
    await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible();
    await page.keyboard.press('Escape');

    await chooseOverflowAction(page, 'About this playground');
    await expect(
      page.getByRole('dialog', { name: 'About this playground' }),
    ).toBeVisible();
  });
});

test.describe('mobile authoring workspace', () => {
  test.use({ viewport: { width: 320, height: 720 } });

  test('uses tabs and mobile dialogs without page scrolling', async ({
    page,
  }) => {
    await openPlayground(page);
    const editorBounds = await page.locator('.cm-editor').boundingBox();
    if (!editorBounds) throw new Error('mobile editor bounds missing');
    const viewportHeight = page.viewportSize()?.height ?? 720;
    expect(
      Math.abs(editorBounds.y + editorBounds.height - viewportHeight),
    ).toBeLessThanOrEqual(4);
    await page.getByRole('tab', { name: 'Preview' }).click();
    await expect(page.locator('.playground-preview-host')).toBeVisible();

    await page.getByRole('button', { name: 'Outline' }).click();
    await expect(page.getByRole('dialog', { name: 'Outline' })).toBeVisible();
    await expectNoAxeViolations(page);
    await page.keyboard.press('Escape');

    await page.getByRole('tab', { name: 'Editor' }).click();
    await replaceEditor(page, '😀》');
    await page.getByRole('button', { name: 'Diagnostics (1)' }).click();
    const diagnostics = page.getByRole('dialog', {
      name: 'Diagnostics (1)',
    });
    await diagnostics
      .getByRole('button', { name: /close bracket has no matching open/i })
      .click();
    await expect(page.getByRole('tab', { name: 'Editor' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    await expect(page.locator('.cm-content')).toBeFocused();
    expect(await page.evaluate(() => getSelection()?.toString())).toBe('》');

    const dimensions = await page.evaluate(() => ({
      body: document.body.scrollHeight,
      document: document.documentElement.scrollHeight,
      viewport: window.innerHeight,
    }));
    expect(dimensions.body).toBeLessThanOrEqual(dimensions.viewport);
    expect(dimensions.document).toBeLessThanOrEqual(dimensions.viewport);
  });

  test('ships a stable 320px visual state', async ({ page }) => {
    await openPlayground(page);
    await page.evaluate(() => document.fonts.ready);
    await expect(page).toHaveScreenshot('mobile-320.png', {
      animations: 'disabled',
      maxDiffPixelRatio: 0.01,
    });
  });
});

test.describe('boot compatibility and production policy', () => {
  test('restores legacy text and compressed query URLs', async ({ page }) => {
    const text = '旧text URL';
    const encoded = Buffer.from(text, 'utf8')
      .toString('base64')
      .replaceAll('+', '-')
      .replaceAll('/', '_')
      .replace(/=+$/, '');
    await page.goto(`./?text=${encoded}`);
    await expect(page.locator('.cm-content')).toContainText(text);

    const compressed = LZString.compressToBase64('旧compressed URL')
      .replaceAll('+', '-')
      .replaceAll('/', '_')
      .replace(/=+$/, '');
    await page.goto(`./?c=${compressed}`);
    await expect(page.locator('.cm-content')).toContainText('旧compressed URL');
  });

  test('gives explicit shared source priority over the saved draft', async ({
    page,
  }) => {
    await page.addInitScript(() => {
      localStorage.setItem('aozora-playground:draft:v1:aozora', 'local draft');
    });
    await page.goto(
      `./#src=${LZString.compressToEncodedURIComponent('shared source')}`,
    );
    await expect(page.locator('.cm-content')).toContainText('shared source');
    await expect(page.locator('.cm-content')).not.toContainText('local draft');
  });

  test('migrates old Aozora draft and shared display preferences', async ({
    page,
  }) => {
    await page.addInitScript(() => {
      localStorage.setItem('aozora-playground:source:v1', 'legacy draft');
      localStorage.setItem('aozora-playground:theme', 'dark');
      localStorage.setItem('aozora-playground:locale', 'ja');
    });
    await page.goto('./');
    await expect(page.locator('.cm-content')).toContainText('legacy draft');
    await expect(page.locator('html')).toHaveAttribute('lang', 'ja');
    await expect(page.locator('html')).toHaveAttribute(
      'data-color-scheme',
      'dark',
    );
  });

  test('recovers when the first WASM request fails', async ({ page }) => {
    let failed = false;
    await page.route('**/*.wasm', async (route) => {
      if (!failed) {
        failed = true;
        await route.abort();
      } else {
        await route.continue();
      }
    });
    await page.goto('./');
    await expect(
      page.getByText('WebAssembly failed to initialize.'),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Retry' }).click();
    await expect(page.locator('.cm-editor')).toBeVisible();
    expect(clientErrors.get(page)).toContain(
      'Failed to load resource: net::ERR_FAILED',
    );
    clientErrors.set(page, []);
  });

  test('keeps CSP, base paths, and every subresource self-hosted', async ({
    page,
  }) => {
    const origins = new Set<string>();
    page.on('request', (request) => origins.add(new URL(request.url()).origin));
    await openPlayground(page);

    const csp = await page
      .locator('meta[http-equiv="Content-Security-Policy"]')
      .getAttribute('content');
    expect(csp).toContain("default-src 'self'");
    expect(csp).toContain("script-src 'self' 'wasm-unsafe-eval'");
    expect(csp).not.toContain('frame-ancestors');
    expect(
      await page.locator('script[type="module"]').getAttribute('src'),
    ).toMatch(/^\/aozora\/playground\/assets\//);
    expect([...origins]).toEqual([new URL(page.url()).origin]);
  });
});
