import type {
  ColorSchemePreference,
  LayoutMode,
  Locale,
  PlaygroundSetting,
  WritingDirection,
} from './types';

const PREFERENCES_KEY = 'aozora-playground:preferences:v2';
const MIGRATION_KEY = 'aozora-playground:migrated:v2';
const DRAFT_PREFIX = 'aozora-playground:draft:v1:';
const SETTINGS_PREFIX = 'aozora-playground:settings:v1:';

export interface UserPreferences {
  colorScheme: ColorSchemePreference;
  locale: Locale;
  layout: LayoutMode;
  writingDirection: WritingDirection;
  outlineOpen: boolean;
}

function storage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

function read(key: string): string | null {
  try {
    return storage()?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

function write(key: string, value: string): boolean {
  try {
    storage()?.setItem(key, value);
    return storage() !== null;
  } catch {
    return false;
  }
}

function isLocale(value: unknown): value is Locale {
  return value === 'ja' || value === 'en';
}

function isColorScheme(value: unknown): value is ColorSchemePreference {
  return value === 'auto' || value === 'light' || value === 'dark';
}

function isLayout(value: unknown): value is LayoutMode {
  return value === 'editor' || value === 'split' || value === 'preview';
}

function isDirection(value: unknown): value is WritingDirection {
  return value === 'horizontal' || value === 'vertical';
}

function navigatorLocale(): Locale {
  return (globalThis.navigator?.language ?? 'ja').toLowerCase().startsWith('en')
    ? 'en'
    : 'ja';
}

function queryLocale(): Locale | null {
  try {
    const value = new URL(globalThis.location.href).searchParams.get('lang');
    return isLocale(value) ? value : null;
  } catch {
    return null;
  }
}

export function defaultPreferences(): UserPreferences {
  return {
    colorScheme: 'auto',
    locale: navigatorLocale(),
    layout: 'split',
    writingDirection: 'horizontal',
    outlineOpen: false,
  };
}

function readLegacyPreferences(): Partial<UserPreferences> {
  const migrated: Partial<UserPreferences> = {};

  const colorScheme =
    read('aozora-md-playground:color-scheme') ??
    read('aozora-playground:theme');
  if (isColorScheme(colorScheme)) migrated.colorScheme = colorScheme;

  const locale = read('aozora-playground:locale');
  if (isLocale(locale)) migrated.locale = locale;

  const direction = read('aozora-md-playground:theme-mode');
  if (isDirection(direction)) migrated.writingDirection = direction;

  const layout = read('aozora-playground:layout');
  if (isLayout(layout)) migrated.layout = layout;

  return migrated;
}

export function loadPreferences(): UserPreferences {
  const defaults = defaultPreferences();
  const storedPreferences = read(PREFERENCES_KEY);
  let parsed: Partial<UserPreferences> = {};
  try {
    parsed = JSON.parse(storedPreferences ?? '{}') as Partial<UserPreferences>;
  } catch {
    parsed = {};
  }
  const migrationPending =
    storedPreferences === null || read(MIGRATION_KEY) === null;
  const legacy = migrationPending ? readLegacyPreferences() : {};
  const combined = { ...defaults, ...legacy, ...parsed };
  const preferences = {
    colorScheme: isColorScheme(combined.colorScheme)
      ? combined.colorScheme
      : defaults.colorScheme,
    locale:
      queryLocale() ??
      (isLocale(combined.locale) ? combined.locale : defaults.locale),
    layout: isLayout(combined.layout) ? combined.layout : defaults.layout,
    writingDirection: isDirection(combined.writingDirection)
      ? combined.writingDirection
      : defaults.writingDirection,
    outlineOpen:
      typeof combined.outlineOpen === 'boolean'
        ? combined.outlineOpen
        : defaults.outlineOpen,
  };
  if (
    migrationPending &&
    write(PREFERENCES_KEY, JSON.stringify(preferences))
  ) {
    write(MIGRATION_KEY, '1');
  }
  return preferences;
}

export function savePreferences(preferences: UserPreferences): boolean {
  return write(PREFERENCES_KEY, JSON.stringify(preferences));
}

export function loadDraft(productId: string): string | null {
  const current = read(`${DRAFT_PREFIX}${productId}`);
  if (current !== null) return current;
  if (productId === 'aozora') {
    return read('aozora-playground:source:v1');
  }
  return null;
}

export function saveDraft(productId: string, source: string): boolean {
  return write(`${DRAFT_PREFIX}${productId}`, source);
}

export function loadSettingValues(
  productId: string,
  settings: readonly PlaygroundSetting[],
): Record<string, boolean> {
  let stored: Record<string, unknown> = {};
  try {
    const parsed: unknown = JSON.parse(
      read(`${SETTINGS_PREFIX}${productId}`) ?? '{}',
    );
    if (
      typeof parsed === 'object' &&
      parsed !== null &&
      !Array.isArray(parsed)
    ) {
      stored = parsed as Record<string, unknown>;
    }
  } catch {
    stored = {};
  }
  const values: Record<string, boolean> = {};
  for (const setting of settings) {
    const storedValue = stored[setting.id];
    values[setting.id] =
      typeof storedValue === 'boolean' ? storedValue : setting.defaultValue;
  }
  return values;
}

export function saveSettingValues(
  productId: string,
  values: Readonly<Record<string, boolean>>,
): boolean {
  return write(`${SETTINGS_PREFIX}${productId}`, JSON.stringify(values));
}
