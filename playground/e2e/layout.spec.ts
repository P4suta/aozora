import { test, expect, type Page } from '@playwright/test';

// Fit-to-viewport layout invariants (Part B ③). Generalises the vertical-scroll
// regression that smoke.spec.ts pins for the tategaki case: the whole chain
// `.app` (definite height) → `.app-main` (flex:1, min-height:0) → panes must
// keep every layout mode inside the viewport, so the page itself never scrolls
// and each pane clips its own overflow. smoke.spec.ts already covers the
// vertical preview + inline-axis scroll, so this file owns only the net-new
// invariants (desktop + short mobile, split-pane bottoms) and does NOT repeat
// the vertical case.

// Engine-ready gate (mirrors smoke.spec.ts): the "WASM 初期化中…" banner is a
// Solid <Show> that unmounts once the engine loads; a fatal load surfaces
// `.error-banner-critical` instead. Both banners are `flex:0 0 auto` and would
// perturb the height maths, so wait them out before measuring.
async function ready(page: Page): Promise<void> {
  await expect(page.locator('.status-banner')).toHaveCount(0, { timeout: 30_000 });
  await expect(page.locator('.error-banner-critical')).toHaveCount(0);
}

const VIEWPORTS = [
  // Desktop: side-by-side split.
  { name: 'desktop', width: 1280, height: 720 },
  // Short mobile: width < 760 trips the mobile @media that stacks the panes as
  // two `minmax(0, 1fr)` rows — the interesting failure mode where stacked
  // panes could burst the `overflow:hidden` .app on a short viewport.
  { name: 'mobile-short', width: 667, height: 375 },
];

for (const vp of VIEWPORTS) {
  test.describe(`layout @ ${vp.name} (${vp.width}×${vp.height})`, () => {
    test.use({ viewport: { width: vp.width, height: vp.height } });

    // The app boots in split mode (App.tsx layoutMode default, not persisted)
    // with default sample text, so no interaction/typing is needed to measure.
    test('ページ（ドキュメント）自体はスクロールしない', async ({ page }) => {
      await page.goto('/');
      await ready(page);
      await expect(page.locator('main.app-main')).toHaveClass(/mode-split/);
      const overflow = await page.evaluate(
        () => document.documentElement.scrollHeight - window.innerHeight,
      );
      expect(overflow).toBeLessThanOrEqual(1); // sub-pixel 丸め許容
    });

    // Guards the `minmax(0, 1fr)` columns/rows: each pane is sized by the grid
    // (content scrolls *inside* it), so its box must end within the viewport.
    // If the definite-height chain broke, a pane would grow to its content and
    // its bottom would exceed innerHeight.
    test('split の両ペイン下端が viewport 内に収まる', async ({ page }) => {
      await page.goto('/');
      await ready(page);
      const innerH = await page.evaluate(() => window.innerHeight);
      for (const sel of ['.editor-pane', '.preview-pane-wrapper']) {
        const bottom = await page
          .locator(sel)
          .evaluate((el) => el.getBoundingClientRect().bottom);
        expect(bottom, `${sel} の下端が viewport を超えている`).toBeLessThanOrEqual(innerH + 1);
      }
    });
  });
}
