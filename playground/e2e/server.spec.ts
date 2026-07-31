import { expect, test } from '@playwright/test';

test.describe('production server policy', () => {
  test('serves HEAD without a body and preserves representation headers', async ({
    page,
  }) => {
    const documentResponse = await page.request.get('./');
    expect(documentResponse.status()).toBe(200);
    expect(documentResponse.headers()['cache-control']).toBe('no-cache');

    const scriptSource = await documentResponse
      .text()
      .then(
        (html) =>
          html.match(/<script[^>]+type="module"[^>]+src="([^"]+)"/)?.[1],
      );
    expect(scriptSource).toBeTruthy();

    const response = await page.request.fetch(scriptSource ?? '', {
      method: 'HEAD',
      headers: { 'Accept-Encoding': 'gzip' },
    });
    expect(response.status()).toBe(200);
    expect(response.headers()['cache-control']).toContain('immutable');
    expect(response.headers()['content-encoding']).toBe('gzip');
    expect(await response.body()).toHaveLength(0);

    const identityResponse = await page.request.fetch(scriptSource ?? '', {
      method: 'HEAD',
      headers: { 'Accept-Encoding': 'gzip;q=0, identity;q=1' },
    });
    expect(identityResponse.status()).toBe(200);
    expect(identityResponse.headers()['content-encoding']).toBeUndefined();
    expect(identityResponse.headers().vary).toBe('Accept-Encoding');
    expect(await identityResponse.body()).toHaveLength(0);
  });

  test('confines decoded and malformed paths to the production root', async ({
    baseURL,
    page,
  }) => {
    if (!baseURL) throw new Error('Playwright baseURL is required');
    const origin = new URL(baseURL).origin;
    const base = '/aozora/playground/';
    for (const path of [
      `${base}%2e%2e/%2e%2e/Cargo.toml`,
      `${base}%E0%A4%A`,
      '/Cargo.toml',
    ]) {
      const response = await page.request.get(`${origin}${path}`);
      expect(response.status()).toBe(404);
    }
  });
});
