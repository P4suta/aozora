/**
 * シンプルな in-memory localStorage + 最小 location polyfill。
 *
 * vitest 4 では `environment: 'happy-dom'` が global にきれいに inject
 * されないケースがあったため、テスト側で polyfill を自前で持たせる。
 * happy-dom の API 全部は要らず、localStorage の getItem / setItem /
 * removeItem / clear と、location.origin / location.pathname だけが
 * あれば storage.ts と share.ts のテストが回る。
 */

if (typeof globalThis.location === 'undefined') {
  Object.defineProperty(globalThis, 'location', {
    configurable: true,
    value: new URL('https://test.local/aozora/playground/'),
  });
}

if (typeof globalThis.localStorage === 'undefined') {
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem(key: string): string | null {
        return store.has(key) ? store.get(key)! : null;
      },
      setItem(key: string, value: string): void {
        store.set(key, String(value));
      },
      removeItem(key: string): void {
        store.delete(key);
      },
      clear(): void {
        store.clear();
      },
      key(index: number): string | null {
        return Array.from(store.keys())[index] ?? null;
      },
      get length(): number {
        return store.size;
      },
    } as Storage,
  });
}
