// The UI languages the playground ships (#336 D-6). `ja` is the source of
// truth; `en` is the translation. `MessageKey` is derived from the `ja`
// catalogue (see catalog.ts) so the two can never drift.
export type Locale = 'ja' | 'en';
