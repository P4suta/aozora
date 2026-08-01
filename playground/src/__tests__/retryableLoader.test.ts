import { describe, expect, it, vi } from 'vitest';

import { createRetryableLoader } from '../retryableLoader';

describe('createRetryableLoader', () => {
  it('shares an in-flight load and caches its success', async () => {
    let resolveResult!: (value: { ready: true }) => void;
    const result = new Promise<{ ready: true }>((resolve) => {
      resolveResult = resolve;
    });
    const load = vi.fn(() => result);
    const retryable = createRetryableLoader(load);

    const first = retryable();
    const second = retryable();
    expect(first).toBe(second);
    expect(load).not.toHaveBeenCalled();

    resolveResult({ ready: true });
    await expect(first).resolves.toEqual({ ready: true });
    expect(retryable()).toBe(first);
    expect(load).toHaveBeenCalledOnce();
  });

  it('forgets a rejected load so retry can fetch the chunk again', async () => {
    const load = vi
      .fn<() => Promise<string>>()
      .mockRejectedValueOnce(new Error('chunk unavailable'))
      .mockResolvedValueOnce('loaded');
    const retryable = createRetryableLoader(load);

    await expect(retryable()).rejects.toThrow('chunk unavailable');
    await expect(retryable()).resolves.toBe('loaded');
    expect(load).toHaveBeenCalledTimes(2);
  });
});
