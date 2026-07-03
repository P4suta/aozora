import { test, expect, type Page, type Locator } from '@playwright/test';

// Notation-gallery E2E (#399 WS-5). The gallery is a second SPA entry
// (gallery.html / gallery.tsx) that parses one fixture per *visible* notation
// family through the real WASM engine and mounts each into a horizontal
// (`.aozora-notation`) and a vertical (`.aozora-vertical`) preview. This suite
// proves the canonical stylesheet actually *styles* every family in both
// writing modes — not merely that the class token is present — by reading
// computed styles, and that the sheet flips writing-mode per column.
//
// Mirrors smoke.spec.ts's TCY computed-style test: assert the styled property
// via `locator.evaluate((el) => getComputedStyle(el).<prop>)`.

// The gallery mounts only after `ensureWasmReady()` resolves (gallery.tsx), so
// the first rendered `<ruby>` is the readiness signal — the analogue of
// smoke.spec.ts waiting for App.tsx's status banner to unmount.
async function ready(page: Page): Promise<void> {
  // Relative (no leading slash) so it resolves against the configured
  // baseURL's `/aozora/playground/` path — the base the second entry is
  // built under. A leading `/` would hit the origin root (404).
  await page.goto('gallery.html');
  await expect(page.locator('[data-family="ruby"] .gallery-h ruby')).toBeVisible({
    timeout: 30_000,
  });
}

// The first element matching `selector` under a family's horizontal column
// (`.gallery-h`) and its vertical column (`.gallery-v`). Both must be visible.
function columns(page: Page, family: string, selector: string): { h: Locator; v: Locator } {
  return {
    h: page.locator(`[data-family="${family}"] .gallery-h ${selector}`).first(),
    v: page.locator(`[data-family="${family}"] .gallery-v ${selector}`).first(),
  };
}

test.describe('notation gallery', () => {
  test('ルビ: ruby-position が横書き・縦書きの両方で over', async ({ page }) => {
    await ready(page);
    const { h, v } = columns(page, 'ruby', 'ruby');
    await expect(h).toBeVisible();
    await expect(v).toBeVisible();
    expect(await h.evaluate((el) => getComputedStyle(el).rubyPosition)).toBe('over');
    expect(await v.evaluate((el) => getComputedStyle(el).rubyPosition)).toBe('over');
  });

  test('傍点: text-emphasis-style が両モードで sesame', async ({ page }) => {
    await ready(page);
    const { h, v } = columns(page, 'bouten', '.aozora-bouten');
    await expect(h).toBeVisible();
    await expect(v).toBeVisible();
    expect(await h.evaluate((el) => getComputedStyle(el).textEmphasisStyle)).toContain('sesame');
    expect(await v.evaluate((el) => getComputedStyle(el).textEmphasisStyle)).toContain('sesame');
  });

  test('縦中横: text-combine-upright が両モードで all', async ({ page }) => {
    await ready(page);
    const { h, v } = columns(page, 'tcy', '.aozora-combine-upright');
    await expect(h).toBeVisible();
    await expect(v).toBeVisible();
    expect(await h.evaluate((el) => getComputedStyle(el).textCombineUpright)).toBe('all');
    expect(await v.evaluate((el) => getComputedStyle(el).textCombineUpright)).toBe('all');
  });

  test('返り点: vertical-align が両モードで super', async ({ page }) => {
    await ready(page);
    const { h, v } = columns(page, 'kaeriten', '.aozora-kaeriten');
    // A super-aligned kaeriten mark collapses to a zero-area box in vertical
    // writing mode, so assert it is ATTACHED (in the DOM) rather than visible;
    // the computed vertical-align — the actual stylesheet assertion — holds
    // regardless. The horizontal column keeps the stricter visibility check.
    await expect(h).toBeVisible();
    await expect(v).toBeAttached();
    expect(await h.evaluate((el) => getComputedStyle(el).verticalAlign)).toBe('super');
    expect(await v.evaluate((el) => getComputedStyle(el).verticalAlign)).toBe('super');
  });

  test('外字: background-color が両モードで非透過', async ({ page }) => {
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
  test('コンテナの writing-mode が列ごとに horizontal-tb / vertical-rl', async ({ page }) => {
    await ready(page);
    for (const family of ['ruby', 'bouten', 'tcy', 'kaeriten', 'gaiji', 'angle-quote', 'warichu']) {
      const { h, v } = columns(page, family, '.html-preview');
      await expect(h).toBeVisible();
      await expect(v).toBeVisible();
      expect(await h.evaluate((el) => getComputedStyle(el).writingMode)).toBe('horizontal-tb');
      expect(await v.evaluate((el) => getComputedStyle(el).writingMode)).toBe('vertical-rl');
    }
  });
});
