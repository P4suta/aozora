import { test, expect, type Page } from '@playwright/test';
import { SAMPLES } from '../src/samples';

// Ruby-attachment E2E over the real WASM-backed playground. Guards the
// render-leak class the A/B/C/D/E1 classifier fixes closed (#398 + WS-2b):
// quote-interior ruby, gaiji-base ruby, and quote-interior gaiji-base ruby
// must attach into a real <ruby>/<rt> without leaking the source delimiters
// (｜ 《 》 ［＃ ］) into the rendered HTML. Selectors + idioms mirror
// smoke.spec.ts (App.tsx / HtmlPreview.tsx).

// Same readiness signal as smoke.spec.ts: the `.status-banner` ("WASM 初期化中…")
// is unmounted once the engine loads; `.error-banner-critical` on hard failure.
async function ready(page: Page): Promise<void> {
  await page.goto('/');
  await expect(page.locator('.status-banner')).toHaveCount(0, { timeout: 30_000 });
  await expect(page.locator('.error-banner-critical')).toHaveCount(0);
}

// Pull the exact demo string from samples.ts by id so these fixtures never
// drift from the shipped samples; fail loudly if a sample is ever renamed.
function sampleText(id: string): string {
  const found = SAMPLES.find((s) => s.id === id);
  if (!found) throw new Error(`sample '${id}' not found in samples.ts`);
  return found.text;
}

test.describe('ルビ付与（実 WASM 描画・leak 根絶ガード）', () => {
  test('引用符内ルビ（｜ 明示ベース）が leak せず <ruby> に付く', async ({ page }) => {
    await ready(page);
    // samples.ts 'ruby' = 「先生｜雑司ヶ谷《ぞうしがや》…」。ヶ で自動ベース検出が
    // 止まるため ｜ で語全体をベース化する。ベース=雑司ヶ谷 / 読み=ぞうしがや。
    await page.getByRole('tab', { name: 'HTML preview' }).click();
    const editor = page.locator('.cm-host .cm-content');
    await editor.click();
    await page.keyboard.press('ControlOrMeta+a');
    await editor.pressSequentially(sampleText('ruby'));

    const ruby = page.locator('.html-preview ruby');
    await expect(ruby).toContainText('雑司ヶ谷');
    await expect(ruby.locator('rt')).toContainText('ぞうしがや');

    // 分類器が閉じる前は ｜ / 《 が地の文へ leak していた（本命の回帰ガード）。
    const preview = page.locator('.html-preview');
    await expect(preview).not.toContainText('｜');
    await expect(preview).not.toContainText('《');
  });

  test('外字ベースルビが .aozora-gaiji ベース＋rt で付き leak しない', async ({ page }) => {
    await ready(page);
    // samples.ts 'gaiji' = 美女、瞳を※［＃「目＋爭」、第3水準1-88-85］《みは》る。
    // ※［＃…］ の外字がルビのベース、《みは》がその読み。
    await page.getByRole('tab', { name: 'HTML preview' }).click();
    const editor = page.locator('.cm-host .cm-content');
    await editor.click();
    await page.keyboard.press('ControlOrMeta+a');
    await editor.pressSequentially(sampleText('gaiji'));

    // ベースは置換後グリフのテキストではなく .aozora-gaiji クラスで検証する。
    await expect(page.locator('.html-preview ruby .aozora-gaiji')).toHaveCount(1);
    await expect(page.locator('.html-preview ruby rt')).toContainText('みは');

    const preview = page.locator('.html-preview');
    await expect(preview).not.toContainText('［＃');
    await expect(preview).not.toContainText('《');
  });

  test('引用符内・外字ベースルビ（WS-2b）が leak せず付く', async ({ page }) => {
    await ready(page);
    // WS-2b 専用ガード: 台詞（「」）の内側で外字がルビベースになるケース。
    // ※［＃「くさかんむり／弓」…］《せんきゅう》 → <ruby>芎<rt>せんきゅう</rt></ruby>。
    const text = '「川※［＃「くさかんむり／弓」、第3水準1-90-62］《せんきゅう》といふ」';
    await page.getByRole('tab', { name: 'HTML preview' }).click();
    const editor = page.locator('.cm-host .cm-content');
    await editor.click();
    await page.keyboard.press('ControlOrMeta+a');
    await editor.pressSequentially(text);

    await expect(page.locator('.html-preview ruby .aozora-gaiji')).toHaveCount(1);
    await expect(page.locator('.html-preview ruby rt')).toContainText('せんきゅう');

    const preview = page.locator('.html-preview');
    await expect(preview).not.toContainText('［＃');
    await expect(preview).not.toContainText('《');
  });
});
