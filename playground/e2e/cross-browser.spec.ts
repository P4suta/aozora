import { expect, type Page, test } from '@playwright/test';

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
  // Spectrum's responsive SVG radius is valid CSS geometry but still logged
  // as an attribute parse error by Firefox and WebKit.
  const unexpected = (clientErrors.get(page) ?? []).filter(
    (message) =>
      !message.includes(
        'Invalid value for <circle> attribute r="calc(50% - 0.09375rem)"',
      ),
  );
  expect(unexpected).toEqual([]);
});

test('authors and renders with the real engine', async ({ page }) => {
  await page.goto('./');
  const editor = page.locator('.cm-content');
  await expect(editor).toBeVisible({ timeout: 30_000 });
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText('明治33［＃「33」は縦中横］年');
  await page.getByRole('radio', { name: 'Vertical' }).click();

  const tcy = page.locator('.playground-preview-host .aozora-combine-upright');
  await expect(tcy).toContainText('33');
  expect(
    await tcy.evaluate(
      (element) => getComputedStyle(element).textCombineUpright,
    ),
  ).toBe('all');
});

test('keeps one editor session across responsive layouts', async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 720 });
  await page.goto('./');
  const editor = page.locator('.cm-content');
  const editorShell = page.locator('.cm-editor');
  await expect(editor).toBeVisible({ timeout: 30_000 });
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText('cross-browser history marker');
  await page.keyboard.press('Shift+Home');
  const originalEditor = await editorShell.elementHandle();

  await page.getByRole('tab', { name: 'Preview' }).click();
  await expect(editorShell).toBeHidden();
  await page.getByRole('tab', { name: 'Editor' }).click();
  await expect(editorShell).toBeVisible();

  await page.setViewportSize({ width: 900, height: 800 });
  await expect(page.getByRole('tab', { name: 'Editor' })).toHaveCount(0);
  await page.setViewportSize({ width: 320, height: 720 });
  await expect(page.getByRole('tab', { name: 'Editor' })).toBeVisible();

  const returnedEditor = await editorShell.elementHandle();
  expect(
    await originalEditor?.evaluate(
      (original, returned) => original.isSameNode(returned),
      returnedEditor,
    ),
  ).toBe(true);
  await editor.focus();
  expect(await page.evaluate(() => getSelection()?.toString())).toBe(
    'cross-browser history marker',
  );
  await page.keyboard.press('ControlOrMeta+Z');
  await expect(editor).not.toContainText('cross-browser history marker');
});

test('keeps the real gallery responsive at 320px', async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 720 });
  await page.goto('gallery.html');
  await expect(page.locator('[data-family]')).toHaveCount(7, {
    timeout: 30_000,
  });
  await expect(
    page.locator('[data-family="ruby"] .gallery-v ruby'),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    ),
  ).toBeLessThanOrEqual(1);
});
