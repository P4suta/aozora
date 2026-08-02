export function createRetryableLoader<T>(
  load: () => Promise<T>,
): () => Promise<T> {
  let cached: Promise<T> | null = null;

  return () => {
    if (cached) return cached;

    const attempt = Promise.resolve().then(load);
    cached = attempt;
    void attempt.catch(() => {
      if (cached === attempt) cached = null;
    });
    return attempt;
  };
}
