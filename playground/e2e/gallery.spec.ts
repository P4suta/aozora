import AxeBuilder from '@axe-core/playwright';
import { expect, type Locator, type Page, test } from '@playwright/test';

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

async function ready(page: Page): Promise<void> {
  await page.goto('gallery.html');
  await expect(
    page.locator('[data-family="ruby"] .gallery-h ruby'),
  ).toBeVisible({
    timeout: 30_000,
  });
}

function columns(
  page: Page,
  family: string,
  selector: string,
): { h: Locator; v: Locator } {
  return {
    h: page.locator(`[data-family="${family}"] .gallery-h ${selector}`).first(),
    v: page.locator(`[data-family="${family}"] .gallery-v ${selector}`).first(),
  };
}

test.describe('notation gallery', () => {
  test('ruby uses ruby-position over in both writing modes', async ({
    page,
  }) => {
    await ready(page);
    const { h, v } = columns(page, 'ruby', 'ruby');
    await expect(h).toBeVisible();
    await expect(v).toBeVisible();
    expect(await h.evaluate((el) => getComputedStyle(el).rubyPosition)).toBe(
      'over',
    );
    expect(await v.evaluate((el) => getComputedStyle(el).rubyPosition)).toBe(
      'over',
    );
  });

  test('emphasis dots use sesame in both writing modes', async ({ page }) => {
    await ready(page);
    const { h, v } = columns(page, 'bouten', '.aozora-bouten');
    await expect(h).toBeVisible();
    await expect(v).toBeVisible();
    expect(
      await h.evaluate((el) => getComputedStyle(el).textEmphasisStyle),
    ).toContain('sesame');
    expect(
      await v.evaluate((el) => getComputedStyle(el).textEmphasisStyle),
    ).toContain('sesame');
  });

  test('tate-chu-yoko uses text-combine-upright in both writing modes', async ({
    page,
  }) => {
    await ready(page);
    const { h, v } = columns(page, 'tcy', '.aozora-combine-upright');
    await expect(h).toBeVisible();
    await expect(v).toBeVisible();
    expect(
      await h.evaluate((el) => getComputedStyle(el).textCombineUpright),
    ).toBe('all');
    expect(
      await v.evaluate((el) => getComputedStyle(el).textCombineUpright),
    ).toBe('all');
  });

  test('kaeriten uses vertical-align super in both writing modes', async ({
    page,
  }) => {
    await ready(page);
    const { h, v } = columns(page, 'kaeriten', '.aozora-kaeriten');
    // A super-aligned kaeriten mark collapses to a zero-area box in vertical
    // writing mode, so assert it is ATTACHED (in the DOM) rather than visible;
    // the computed vertical-align — the actual stylesheet assertion — holds
    // regardless. The horizontal column keeps the stricter visibility check.
    await expect(h).toBeVisible();
    await expect(v).toBeAttached();
    expect(await h.evaluate((el) => getComputedStyle(el).verticalAlign)).toBe(
      'super',
    );
    expect(await v.evaluate((el) => getComputedStyle(el).verticalAlign)).toBe(
      'super',
    );
  });

  test('gaiji has an opaque background in both writing modes', async ({
    page,
  }) => {
    await ready(page);
    const { h, v } = columns(page, 'gaiji', '.aozora-gaiji');
    await expect(h).toBeVisible();
    await expect(v).toBeVisible();
    const transparent = ['rgba(0, 0, 0, 0)', 'transparent'];
    const bgH = await h.evaluate((el) => getComputedStyle(el).backgroundColor);
    const bgV = await v.evaluate((el) => getComputedStyle(el).backgroundColor);
    expect(transparent).not.toContain(bgH);
    expect(transparent).not.toContain(bgV);
  });

  // The canonical sheet must set the writing mode per column so both previews
  // exercise it: the horizontal container stays `horizontal-tb`, the vertical
  // one becomes `vertical-rl`. Checked on every family's `.html-preview`
  // container to prove the scope hooks apply uniformly.
  test('columns use horizontal-tb and vertical-rl writing modes', async ({
    page,
  }) => {
    await ready(page);
    for (const family of [
      'ruby',
      'bouten',
      'tcy',
      'kaeriten',
      'gaiji',
      'angle-quote',
      'warichu',
    ]) {
      const { h, v } = columns(page, family, '.html-preview');
      await expect(h).toBeVisible();
      await expect(v).toBeVisible();
      expect(await h.evaluate((el) => getComputedStyle(el).writingMode)).toBe(
        'horizontal-tb',
      );
      expect(await v.evaluate((el) => getComputedStyle(el).writingMode)).toBe(
        'vertical-rl',
      );
    }
  });

  test('real renderer output has no WCAG 2.2 AA violations in either mode', async ({
    page,
  }) => {
    await ready(page);
    await expect(page.locator('[data-family]')).toHaveCount(7);
    await expect(page.locator('.gallery-h .html-preview')).toHaveCount(7);
    await expect(page.locator('.gallery-v .html-preview')).toHaveCount(7);
    expect(
      (await page.locator('.html-preview').allTextContents()).join(''),
    ).not.toContain('［＃');
    const results = await new AxeBuilder({ page })
      .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(results.violations).toEqual([]);
  });

  test('all notation output meets WCAG 2.2 AA in the dark theme', async ({
    page,
  }) => {
    await page.addInitScript(() => {
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
    await ready(page);
    await expect(page.locator('html')).toHaveAttribute(
      'data-color-scheme',
      'dark',
    );
    const results = await new AxeBuilder({ page })
      .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(results.violations).toEqual([]);
  });

  test('320px viewport uses one column without page overflow', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 320, height: 720 });
    await ready(page);
    const columns = await page
      .locator('[data-family="ruby"] .gallery-columns')
      .evaluate((element) => getComputedStyle(element).gridTemplateColumns);
    expect(columns.trim().split(/\s+/)).toHaveLength(1);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth - window.innerWidth,
      ),
    ).toBeLessThanOrEqual(1);
  });

  test('keeps a stable gallery rendering', async ({ page }) => {
    await ready(page);
    await page.evaluate(() => document.fonts.ready);
    await expect(page).toHaveScreenshot('gallery-default.png', {
      animations: 'disabled',
      fullPage: true,
      maxDiffPixelRatio: 0.01,
    });
  });
});
