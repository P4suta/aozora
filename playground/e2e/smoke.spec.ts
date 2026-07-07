import { test, expect, type Page } from '@playwright/test';

// Smoke E2E over the real WASM-backed playground (#335 D-5). Selectors verified
// against the live components (App.tsx / HtmlPreview.tsx / Tabs.tsx /
// SettingsPanel.tsx / NotationGuide.tsx).

// The `.status-banner` ("WASM 初期化中…") is rendered by a Solid `<Show>` while
// `!wasmReady()`, so it is *unmounted* once the engine loads — the canonical
// readiness signal (App.tsx). A critical load failure surfaces
// `.error-banner-critical` instead.
async function ready(page: Page): Promise<void> {
  await page.goto('/');
  await expect(page.locator('.status-banner')).toHaveCount(0, { timeout: 30_000 });
  await expect(page.locator('.error-banner-critical')).toHaveCount(0);
}

test.describe('playground smoke', () => {
  test('WASM 初期化後にエディタとプレビューが表示される', async ({ page }) => {
    await ready(page);
    await expect(page.locator('.cm-host .cm-content')).toBeVisible();
    await expect(page.locator('.html-preview')).toBeVisible();
  });

  test('入力するとプレビュー HTML が更新される（実 WASM 往復）', async ({ page }) => {
    await ready(page);
    // The HTML-preview tab is index 0 and persists to localStorage; a fresh
    // context starts clean, but click it explicitly to be deterministic.
    await page.getByRole('tab', { name: 'HTML preview' }).click();
    const editor = page.locator('.cm-host .cm-content');
    await editor.click();
    await page.keyboard.press('ControlOrMeta+a');
    await editor.pressSequentially('猫《ねこ》');
    await expect(page.locator('.html-preview ruby')).toContainText('猫');
  });

  test('縦書きトグルでプレビューが is-vertical になる', async ({ page }) => {
    await ready(page);
    await page.getByRole('tab', { name: 'HTML preview' }).click();
    const preview = page.locator('.html-preview');
    await expect(preview).not.toHaveClass(/is-vertical/);
    await page.locator('.writing-mode-btn').click();
    await expect(preview).toHaveClass(/is-vertical/);
  });

  test('縦中横(TCY)が縦書きで text-combine-upright: all で合成される', async ({ page }) => {
    // ユーザー報告の回帰ガード: 正準スタイルシートが `.aozora-combine-upright`
    // に `text-combine-upright: all` を当てていないと、縦書きで半角数字が縦積みに
    // なる。クラス付与だけでなく computed-style を検証する。
    await ready(page);
    await page.getByRole('tab', { name: 'HTML preview' }).click();
    const editor = page.locator('.cm-host .cm-content');
    await editor.click();
    await page.keyboard.press('ControlOrMeta+a');
    // ruby テストと同じ pressSequentially。エディタは全角括弧をオートクローズ
    // するが、閉じ括弧を打つと type-over で吸収されるので素直に全文を打てる。
    await editor.pressSequentially('明治33［＃「33」は縦中横］年');

    const tcy = page.locator('.html-preview .aozora-combine-upright');
    await expect(tcy).toContainText('33');

    // 縦書きに切り替え、縦中横が実際に合成される computed-style を確認。
    await page.locator('.writing-mode-btn').click();
    await expect(page.locator('.html-preview')).toHaveClass(/aozora-vertical/);
    const combine = await tcy.evaluate((el) => getComputedStyle(el).textCombineUpright);
    expect(combine).toBe('all');
  });

  test('縦書きで長文を入力してもページはスクロールせずプレビュー内で横スクロールする', async ({
    page,
  }) => {
    // ユーザー報告の回帰ガード: 縦書き(vertical-rl)で長文を打つと、.app の高さが
    // 不確定だとプレビューが縦(実際は横)に伸び続け、body スクロールが発生していた。
    // .app を確定高さ(100dvh)の flex column にしたことで、溢れはプレビュー枠内の
    // 横スクロール(inline 軸)に閉じ込められ、ページ自体は伸びない。
    await page.setViewportSize({ width: 700, height: 480 });
    await ready(page);
    await page.getByRole('tab', { name: 'HTML preview' }).click();
    await page.locator('.writing-mode-btn').click();
    const preview = page.locator('.html-preview.is-vertical');
    await expect(preview).toBeVisible();

    const editor = page.locator('.cm-host .cm-content');
    await editor.click();
    await page.keyboard.press('ControlOrMeta+a');
    const longText = 'あの日見た花の名前を僕達はまだ知らない。'.repeat(30);
    await editor.pressSequentially(longText);
    // プレビューの再描画が落ち着いてから計測する(空だと scrollWidth の判定が偽陰性)。
    await expect(preview).toContainText('あの日見た花の名前');

    // (1) ページ(ドキュメント)自体はスクロールしない。
    const pageOverflow = await page.evaluate(
      () => document.documentElement.scrollHeight - window.innerHeight,
    );
    expect(pageOverflow).toBeLessThanOrEqual(1); // sub-pixel の丸め許容

    // (2) 溢れはプレビュー枠内へ閉じ込められ、かつ横(inline 軸)が実際にスクロール可能。
    //     scrollWidth>clientWidth(溢れの事実)だけだと overflow-x:hidden でも真になり
    //     「クリップされて読めない」退行を見逃すので、computed overflow-x も検証する。
    const { overflows, overflowX } = await preview.evaluate((el) => ({
      overflows: el.scrollWidth > el.clientWidth + 1,
      overflowX: getComputedStyle(el).overflowX,
    }));
    expect(overflows).toBe(true); // 枠は content より狭い = ページを押し広げていない
    expect(['auto', 'scroll']).toContain(overflowX); // ユーザーが実際に横スクロールできる
  });

  test('レイアウトボタンで表示モードが切り替わる', async ({ page }) => {
    await ready(page);
    // First `.layout-btn` is editor-only (⌨) → `main.app-main.mode-editor`.
    await page.locator('.layout-btn').first().click();
    await expect(page.locator('main.app-main')).toHaveClass(/mode-editor/);
  });

  test('記法ガイドが開き Escape で閉じる', async ({ page }) => {
    await ready(page);
    await page.locator('.guide-btn').click();
    await expect(page.locator('[role="dialog"]')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('[role="dialog"]')).toHaveCount(0);
  });

  test('設定ポップオーバーが開く', async ({ page }) => {
    await ready(page);
    await page.locator('.settings-trigger').click();
    await expect(page.locator('.settings-popover')).toBeVisible();
  });

  test('共有ボタンでトーストが出る', async ({ page }) => {
    await ready(page);
    // `copyShareUrl` flashes a toast on every path (success / clipboard-fail /
    // too-long), so this is robust even in a headless clipboard-less context.
    await page.locator('.share-btn').click();
    await expect(page.locator('.toast')).toBeVisible();
  });

  test('プレビュータブを切り替えられる', async ({ page }) => {
    await ready(page);
    const nodesTab = page.getByRole('tab', { name: 'Nodes (JSON)' });
    await nodesTab.click();
    await expect(nodesTab).toHaveAttribute('aria-selected', 'true');
  });

  test('コマンドパレットが開き絞り込みと Escape で閉じる', async ({ page }) => {
    await ready(page);
    await page.locator('.palette-btn').click();
    const palette = page.locator('.command-palette-modal');
    await expect(palette).toBeVisible();
    // Romaji query hits the command id (`aozora.wrap.chuki`).
    await page.locator('.command-palette-input').fill('chuki');
    await expect(page.locator('.command-palette-item')).toHaveCount(1);
    await page.keyboard.press('Escape');
    await expect(palette).toHaveCount(0);
  });

  test('言語トグルで UI 言語が双方向に切り替わる', async ({ page }) => {
    // Boot language follows navigator.language, so don't assume the initial —
    // exercise the toggle both ways.
    await ready(page);
    await page.locator('.settings-trigger').click();
    await page.locator('input[name="lang-pref"][value="en"]').check();
    await expect(page.locator('.guide-btn .btn-text')).toHaveText('Guide');
    await expect(page.locator('html')).toHaveAttribute('lang', 'en');
    await page.locator('input[name="lang-pref"][value="ja"]').check();
    await expect(page.locator('.guide-btn .btn-text')).toHaveText('記法ガイド');
    await expect(page.locator('html')).toHaveAttribute('lang', 'ja');
  });
});
