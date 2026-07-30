import {
  ActionButton,
  Text as ActionButtonText,
} from '@react-spectrum/s2/ActionButton';
import {
  ActionMenu,
  MenuItem,
  Text as MenuText,
} from '@react-spectrum/s2/ActionMenu';
import { Button } from '@react-spectrum/s2/Button';
import {
  Content,
  Dialog,
  DialogContainer,
  Heading,
} from '@react-spectrum/s2/Dialog';
import {
  Disclosure,
  DisclosurePanel,
  DisclosureTitle,
} from '@react-spectrum/s2/Disclosure';
import { InlineAlert } from '@react-spectrum/s2/InlineAlert';
import AlertDiamond from '@react-spectrum/s2/icons/AlertDiamond';
import AlertTriangle from '@react-spectrum/s2/icons/AlertTriangle';
import Code from '@react-spectrum/s2/icons/Code';
import HelpCircle from '@react-spectrum/s2/icons/HelpCircle';
import InfoCircle from '@react-spectrum/s2/icons/InfoCircle';
import Preview from '@react-spectrum/s2/icons/Preview';
import Settings from '@react-spectrum/s2/icons/Settings';
import Share from '@react-spectrum/s2/icons/Share';
import TextIcon from '@react-spectrum/s2/icons/Text';
import ViewList from '@react-spectrum/s2/icons/ViewList';
import { Link } from '@react-spectrum/s2/Link';
import { Picker, PickerItem } from '@react-spectrum/s2/Picker';
import { ProgressCircle } from '@react-spectrum/s2/ProgressCircle';
import { Provider } from '@react-spectrum/s2/Provider';
import { SearchField } from '@react-spectrum/s2/SearchField';
import {
  SegmentedControl,
  SegmentedControlItem,
} from '@react-spectrum/s2/SegmentedControl';
import { Switch } from '@react-spectrum/s2/Switch';
import { style } from '@react-spectrum/s2/style' with { type: 'macro' };
import { Tab, TabList, TabPanel, Tabs } from '@react-spectrum/s2/Tabs';
import { ToastContainer, ToastQueue } from '@react-spectrum/s2/Toast';
import { ToggleButton } from '@react-spectrum/s2/ToggleButton';
import {
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

import { formatMessage, message } from './catalog';
import {
  copyShareUrl,
  readSharedSource,
  ShareUrlTooLongError,
} from './share';
import {
  loadDraft,
  loadPreferences,
  loadSettingValues,
  saveDraft,
  savePreferences,
  saveSettingValues,
  type UserPreferences,
} from './storage';
import type {
  ColorSchemePreference,
  EditorController,
  LayoutMode,
  Locale,
  PlaygroundAdapter,
  PlaygroundAnalysis,
  PlaygroundAppProps,
  PlaygroundDiagnostic,
  PlaygroundGuide,
  PlaygroundOutlineEntry,
  PreviewController,
  TextRange,
  WritingDirection,
} from './types';

const POST_PAINT_WORK_DELAY_MS = 100;

const shellStyle = style({
  display: 'flex',
  flexDirection: 'column',
  height: 'full',
  minHeight: 0,
  overflow: 'hidden',
});

const headerStyle = style({
  alignItems: 'center',
  backgroundColor: 'layer-1',
  borderBottomWidth: 1,
  borderColor: 'gray-300',
  borderStyle: 'solid',
  columnGap: 8,
  display: 'flex',
  flexShrink: 0,
  minHeight: 56,
  paddingX: 12,
  paddingY: 8,
});

const brandStyle = style({
  display: 'flex',
  flexDirection: 'column',
  flexShrink: 0,
  minWidth: 0,
});

const brandNameStyle = style({
  color: 'neutral',
  font: 'title-sm',
  margin: 0,
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
});

const brandDescriptionStyle = style({
  color: 'gray-700',
  font: 'detail-sm',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
});

const headerActionsStyle = style({
  alignItems: 'center',
  columnGap: 4,
  display: 'flex',
  flexGrow: 1,
  justifyContent: 'end',
  minWidth: 0,
});

const workspaceStyle = style({
  display: 'flex',
  flexGrow: 1,
  minHeight: 0,
  minWidth: 0,
  overflow: 'hidden',
});

const panesStyle = style({
  display: 'grid',
  flexGrow: 1,
  minHeight: 0,
  minWidth: 0,
  overflow: 'hidden',
});

const splitPanesStyle = style({
  gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
});

const singlePaneStyle = style({
  gridTemplateColumns: 'minmax(0, 1fr)',
});

const paneStyle = style({
  display: 'flex',
  flexDirection: 'column',
  flexGrow: 1,
  minHeight: 0,
  minWidth: 0,
  overflow: 'hidden',
});

const editorBorderStyle = style({
  borderEndWidth: 1,
  borderColor: 'gray-300',
  borderStyle: 'solid',
});

const paneHeaderStyle = style({
  alignItems: 'center',
  backgroundColor: 'layer-1',
  borderBottomWidth: 1,
  borderColor: 'gray-300',
  borderStyle: 'solid',
  display: 'flex',
  flexShrink: 0,
  justifyContent: 'space-between',
  minHeight: 48,
  paddingX: 12,
  paddingY: 4,
});

const paneTitleStyle = style({
  color: 'neutral',
  font: 'title-sm',
});

const outlineStyle = style({
  backgroundColor: 'layer-1',
  borderEndWidth: 1,
  borderColor: 'gray-300',
  borderStyle: 'solid',
  display: 'flex',
  flexDirection: 'column',
  flexShrink: 0,
  minHeight: 0,
  overflow: 'hidden',
  width: 240,
});

const scrollAreaStyle = style({
  display: 'flex',
  flexDirection: 'column',
  flexGrow: 1,
  minHeight: 0,
  overflow: 'auto',
  padding: 12,
  rowGap: 8,
});

const outlineEntryStyle = style({
  alignSelf: 'stretch',
  flexShrink: 0,
  justifySelf: 'stretch',
  width: 'full',
});

const outlineItemStyle = style({
  display: 'flex',
  flexShrink: 0,
  width: 'full',
});

const emptyStyle = style({
  color: 'gray-700',
  font: 'body-sm',
  padding: 12,
});

const previewHostStyle = style({
  flexGrow: 1,
  minHeight: 0,
  minWidth: 0,
  overflow: 'hidden',
});

const centeredStyle = style({
  alignItems: 'center',
  display: 'flex',
  flexDirection: 'column',
  flexGrow: 1,
  justifyContent: 'center',
  minHeight: 0,
  padding: 24,
  rowGap: 12,
});

const diagnosticsStyle = style({
  backgroundColor: 'layer-1',
  borderTopWidth: 1,
  borderColor: 'gray-300',
  borderStyle: 'solid',
  flexShrink: 0,
  maxHeight: 240,
  overflow: 'auto',
});

const dialogStackStyle = style({
  display: 'flex',
  flexDirection: 'column',
  rowGap: 16,
});

const settingsRowStyle = style({
  alignItems: 'center',
  columnGap: 16,
  display: 'flex',
  justifyContent: 'space-between',
  minWidth: 0,
});

const settingsTextStyle = style({
  display: 'flex',
  flexDirection: 'column',
  minWidth: 0,
  rowGap: 2,
});

const descriptionStyle = style({
  color: 'gray-700',
  font: 'body-sm',
});

const guideExampleStyle = style({
  backgroundColor: 'gray-subtle',
  borderRadius: 'default',
  boxSizing: 'border-box',
  font: 'code',
  margin: 0,
  overflow: 'auto',
  padding: 12,
  whiteSpace: 'pre-wrap',
  width: 'full',
});

const mobileToolsStyle = style({
  alignItems: 'center',
  backgroundColor: 'layer-1',
  borderBottomWidth: 1,
  borderColor: 'gray-300',
  borderStyle: 'solid',
  columnGap: 8,
  display: 'flex',
  flexShrink: 0,
  paddingX: 8,
  paddingY: 4,
});

const mobileTabsStyle = style({
  flexBasis: 0,
  flexGrow: 1,
  flexShrink: 1,
  minHeight: 0,
  width: 'full',
});

const mobilePanelStyle = style({
  flexGrow: 1,
  height: 'full',
  minHeight: 0,
});

const mobilePanelContentStyle = style({
  display: 'flex',
  height: 'full',
  minHeight: 0,
});

const EMPTY_ANALYSIS: PlaygroundAnalysis = {
  html: '',
  diagnostics: [],
  outline: [],
};

type InitializationState = 'loading' | 'ready' | 'failed';
type DialogName =
  | 'about'
  | 'commands'
  | 'diagnostics'
  | 'guide'
  | 'outline'
  | 'settings'
  | null;

function localized(
  value: { readonly ja: string; readonly en: string },
  locale: Locale,
): string {
  return value[locale];
}

function afterDialogsClose(action: () => void): () => void {
  let attempts = 0;
  let timer = 0;
  const run = () => {
    if (document.querySelector('[role="dialog"]') !== null && attempts < 100) {
      attempts++;
      timer = globalThis.setTimeout(run, 16);
      return;
    }
    action();
  };
  timer = globalThis.setTimeout(run, 0);
  return () => globalThis.clearTimeout(timer);
}

function useMedia(query: string): boolean {
  const [matches, setMatches] = useState(
    () => globalThis.matchMedia?.(query).matches ?? false,
  );
  useEffect(() => {
    const media = globalThis.matchMedia?.(query);
    if (!media) return;
    const update = () => setMatches(media.matches);
    update();
    media.addEventListener('change', update);
    return () => media.removeEventListener('change', update);
  }, [query]);
  return matches;
}

function useEffectiveColorScheme(
  preference: ColorSchemePreference,
): 'light' | 'dark' {
  const systemDark = useMedia('(prefers-color-scheme: dark)');
  return preference === 'auto' ? (systemDark ? 'dark' : 'light') : preference;
}

function initialSource(adapter: PlaygroundAdapter): {
  readonly source: string;
  readonly invalidUrl: boolean;
} {
  const shared = readSharedSource(new URL(globalThis.location.href));
  if (shared.status === 'ok') {
    return { source: shared.source, invalidUrl: false };
  }
  const draft = loadDraft(adapter.product.id);
  if (draft !== null) {
    return { source: draft, invalidUrl: shared.status === 'invalid' };
  }
  return {
    source: adapter.samples[0]?.source ?? '',
    invalidUrl: shared.status === 'invalid',
  };
}

function EditorMount({
  adapter,
  locale,
  onChange,
  onController,
  settingValues,
  value,
}: {
  readonly adapter: PlaygroundAdapter;
  readonly locale: Locale;
  readonly onChange: (value: string) => void;
  readonly onController: (controller: EditorController | null) => void;
  readonly settingValues: Readonly<Record<string, boolean>>;
  readonly value: string;
}) {
  const mountRef = useRef<HTMLDivElement>(null);
  const controllerRef = useRef<EditorController | null>(null);
  const initialValueRef = useRef(value);
  const latestValueRef = useRef(value);
  const latestSettingValuesRef = useRef(settingValues);
  latestValueRef.current = value;
  latestSettingValuesRef.current = settingValues;

  useEffect(() => {
    const parent = mountRef.current;
    if (!parent) return;
    let disposed = false;
    let secondFrame = 0;
    let timer = 0;
    const mountEditor = () => {
      adapter.setLocale?.(locale);
      void Promise.resolve(
        adapter.createEditor(parent, initialValueRef.current, onChange),
      ).then((controller) => {
        if (disposed) {
          controller.destroy();
          return;
        }
        controllerRef.current = controller;
        controller.setValue(latestValueRef.current);
        for (const [id, enabled] of Object.entries(
          latestSettingValuesRef.current,
        )) {
          controller.setSetting(id, enabled);
        }
        onController(controller);
      });
    };
    const firstFrame = globalThis.requestAnimationFrame(() => {
      secondFrame = globalThis.requestAnimationFrame(() => {
        if (adapter.createEditorDuringInitialization) {
          timer = globalThis.setTimeout(
            mountEditor,
            POST_PAINT_WORK_DELAY_MS,
          );
        } else {
          mountEditor();
        }
      });
    });
    return () => {
      disposed = true;
      globalThis.cancelAnimationFrame(firstFrame);
      globalThis.cancelAnimationFrame(secondFrame);
      globalThis.clearTimeout(timer);
      onController(null);
      controllerRef.current?.destroy();
      controllerRef.current = null;
    };
  }, [adapter, locale, onChange, onController]);

  useEffect(() => {
    controllerRef.current?.setValue(value);
  }, [value]);

  useEffect(() => {
    const controller = controllerRef.current;
    if (!controller) return;
    for (const [id, enabled] of Object.entries(settingValues)) {
      controller.setSetting(id, enabled);
    }
  }, [settingValues]);

  return <div className="playground-editor-host" ref={mountRef} />;
}

function PreviewMount({
  adapter,
  direction,
  html,
}: {
  readonly adapter: PlaygroundAdapter;
  readonly direction: WritingDirection;
  readonly html: string;
}) {
  const mountRef = useRef<HTMLDivElement>(null);
  const controllerRef = useRef<PreviewController | null>(null);
  const initialDirectionRef = useRef(direction);
  const initialHtmlRef = useRef(html);

  useLayoutEffect(() => {
    const parent = mountRef.current;
    if (!parent) return;
    const controller = adapter.createPreview(parent);
    controllerRef.current = controller;
    controller.update(initialHtmlRef.current, initialDirectionRef.current);
    return () => {
      controller.destroy();
      controllerRef.current = null;
    };
  }, [adapter]);

  useEffect(() => {
    controllerRef.current?.update(html, direction);
  }, [direction, html]);

  return (
    <div
      className="playground-preview-host"
      data-writing-direction={direction}
      ref={mountRef}
    />
  );
}

function OutlineList({
  entries,
  locale,
  onJump,
}: {
  readonly entries: readonly PlaygroundOutlineEntry[];
  readonly locale: Locale;
  readonly onJump: (range: TextRange) => void;
}) {
  if (entries.length === 0) {
    return <div className={emptyStyle}>{message(locale, 'outlineEmpty')}</div>;
  }
  return (
    <div className={scrollAreaStyle} role="list">
      {entries.map((entry) => (
        <div
          aria-level={Math.max(1, entry.level)}
          className={outlineItemStyle}
          key={`${entry.level}:${entry.text}:${entry.range?.start ?? 'none'}:${entry.range?.end ?? 'none'}`}
          role="listitem"
        >
          <ActionButton
            aria-label={formatMessage(locale, 'outlineEntryLabel', {
              level: entry.level,
              text: entry.text,
            })}
            isDisabled={entry.range === null}
            onPress={() => {
              if (entry.range) onJump(entry.range);
            }}
            size="S"
            styles={outlineEntryStyle}
          >
            {`${'　'.repeat(Math.max(0, entry.level - 1))}${entry.text}`}
          </ActionButton>
        </div>
      ))}
    </div>
  );
}

function diagnosticLabel(
  diagnostic: PlaygroundDiagnostic,
  locale: Locale,
): string {
  const severity = message(
    locale,
    diagnostic.severity === 'error'
      ? 'diagnosticError'
      : diagnostic.severity === 'warning'
        ? 'diagnosticWarning'
        : 'diagnosticInfo',
  );
  const code = diagnostic.code ? ` (${diagnostic.code})` : '';
  return `${severity}: ${localized(diagnostic.message, locale)}${code}`;
}

function DiagnosticIcon({
  severity,
}: {
  readonly severity: PlaygroundDiagnostic['severity'];
}) {
  if (severity === 'error') return <AlertDiamond />;
  if (severity === 'warning') return <AlertTriangle />;
  return <InfoCircle />;
}

function DiagnosticsList({
  diagnostics,
  locale,
  onJump,
}: {
  readonly diagnostics: readonly PlaygroundDiagnostic[];
  readonly locale: Locale;
  readonly onJump: (range: TextRange) => void;
}) {
  if (diagnostics.length === 0) {
    return (
      <div className={emptyStyle}>{message(locale, 'diagnosticsEmpty')}</div>
    );
  }
  return (
    <div className={scrollAreaStyle}>
      {diagnostics.map((diagnostic) => (
        <ActionButton
          key={`${diagnostic.code ?? diagnostic.severity}:${diagnostic.range.start}:${diagnostic.range.end}:${localized(diagnostic.message, locale)}`}
          onPress={() => onJump(diagnostic.range)}
          size="S"
          styles={outlineEntryStyle}
        >
          <DiagnosticIcon severity={diagnostic.severity} />
          <ActionButtonText>
            {diagnosticLabel(diagnostic, locale)}
          </ActionButtonText>
        </ActionButton>
      ))}
    </div>
  );
}

function DiagnosticsDisclosure({
  diagnostics,
  isExpanded,
  locale,
  onExpandedChange,
  onJump,
}: {
  readonly diagnostics: readonly PlaygroundDiagnostic[];
  readonly isExpanded: boolean;
  readonly locale: Locale;
  readonly onExpandedChange: (expanded: boolean) => void;
  readonly onJump: (range: TextRange) => void;
}) {
  return (
    <div className={diagnosticsStyle}>
      <Disclosure
        isExpanded={isExpanded}
        onExpandedChange={onExpandedChange}
        size="S"
      >
        <DisclosureTitle level={2}>
          {`${message(locale, 'diagnostics')} (${diagnostics.length})`}
        </DisclosureTitle>
        <DisclosurePanel>
          <DiagnosticsList
            diagnostics={diagnostics}
            locale={locale}
            onJump={onJump}
          />
        </DisclosurePanel>
      </Disclosure>
    </div>
  );
}

function GuideContent({
  guide,
  locale,
}: {
  readonly guide: PlaygroundGuide;
  readonly locale: Locale;
}) {
  return (
    <div className={dialogStackStyle}>
      <div className={descriptionStyle}>
        {localized(guide.introduction, locale)}
      </div>
      {guide.sections.map((section) => (
        <section className={dialogStackStyle} key={section.id}>
          <Heading level={3}>{localized(section.title, locale)}</Heading>
          <div>{localized(section.body, locale)}</div>
          {section.example && (
            <pre className={guideExampleStyle}>
              <code>{section.example}</code>
            </pre>
          )}
          {section.href && (
            <Link href={section.href} target="_blank">
              {section.href}
            </Link>
          )}
        </section>
      ))}
    </div>
  );
}

function SettingsContent({
  locale,
  preferences,
  settingValues,
  settings,
  setPreference,
  setSetting,
}: {
  readonly locale: Locale;
  readonly preferences: UserPreferences;
  readonly settingValues: Readonly<Record<string, boolean>>;
  readonly settings: PlaygroundAdapter['settings'];
  readonly setPreference: <Key extends keyof UserPreferences>(
    key: Key,
    value: UserPreferences[Key],
  ) => void;
  readonly setSetting: (id: string, value: boolean) => void;
}) {
  return (
    <div className={dialogStackStyle}>
      <Picker
        label={message(locale, 'theme')}
        onSelectionChange={(key) =>
          setPreference('colorScheme', String(key) as ColorSchemePreference)
        }
        selectedKey={preferences.colorScheme}
      >
        <PickerItem id="auto">{message(locale, 'themeAuto')}</PickerItem>
        <PickerItem id="light">{message(locale, 'themeLight')}</PickerItem>
        <PickerItem id="dark">{message(locale, 'themeDark')}</PickerItem>
      </Picker>
      <Picker
        label={message(locale, 'language')}
        onSelectionChange={(key) =>
          setPreference('locale', String(key) as Locale)
        }
        selectedKey={preferences.locale}
      >
        <PickerItem id="ja">{message(locale, 'languageJapanese')}</PickerItem>
        <PickerItem id="en">{message(locale, 'languageEnglish')}</PickerItem>
      </Picker>
      {settings.map((setting) => (
        <div className={settingsRowStyle} key={setting.id}>
          <div className={settingsTextStyle}>
            <span>{localized(setting.label, locale)}</span>
            <span className={descriptionStyle}>
              {localized(setting.description, locale)}
            </span>
          </div>
          <Switch
            aria-label={localized(setting.label, locale)}
            isSelected={settingValues[setting.id] ?? setting.defaultValue}
            onChange={(value) => setSetting(setting.id, value)}
          />
        </div>
      ))}
    </div>
  );
}

function CommandContent({
  adapter,
  locale,
  onRun,
}: {
  readonly adapter: PlaygroundAdapter;
  readonly locale: Locale;
  readonly onRun: (id: string) => void;
}) {
  const [query, setQuery] = useState('');
  const commands = adapter.commands.filter((command) =>
    localized(command.label, locale)
      .toLocaleLowerCase(locale)
      .includes(query.toLocaleLowerCase(locale)),
  );
  return (
    <div className={dialogStackStyle}>
      <SearchField
        aria-label={message(locale, 'commandSearch')}
        autoFocus
        onChange={setQuery}
        value={query}
      />
      {commands.length === 0 ? (
        <div className={emptyStyle}>{message(locale, 'commandEmpty')}</div>
      ) : (
        commands.map((command) => (
          <ActionButton key={command.id} onPress={() => onRun(command.id)}>
            <Code />
            <ActionButtonText>
              {localized(command.label, locale)}
            </ActionButtonText>
            {command.shortcut && (
              <ActionButtonText slot="description">
                {command.shortcut}
              </ActionButtonText>
            )}
          </ActionButton>
        ))
      )}
    </div>
  );
}

function ActiveDialog({
  adapter,
  dialog,
  locale,
  onDismiss,
  onJump,
  onRunCommand,
  preferences,
  setPreference,
  settingValues,
  setSetting,
  analysis,
}: {
  readonly adapter: PlaygroundAdapter;
  readonly analysis: PlaygroundAnalysis;
  readonly dialog: DialogName;
  readonly locale: Locale;
  readonly onDismiss: () => void;
  readonly onJump: (range: TextRange) => void;
  readonly onRunCommand: (id: string) => void;
  readonly preferences: UserPreferences;
  readonly setPreference: <Key extends keyof UserPreferences>(
    key: Key,
    value: UserPreferences[Key],
  ) => void;
  readonly settingValues: Readonly<Record<string, boolean>>;
  readonly setSetting: (id: string, value: boolean) => void;
}) {
  let title = '';
  let content = null;
  switch (dialog) {
    case 'guide':
      title = localized(adapter.guide.title, locale);
      content = <GuideContent guide={adapter.guide} locale={locale} />;
      break;
    case 'settings':
      title = message(locale, 'settings');
      content = (
        <SettingsContent
          locale={locale}
          preferences={preferences}
          settingValues={settingValues}
          settings={adapter.settings}
          setPreference={setPreference}
          setSetting={setSetting}
        />
      );
      break;
    case 'about':
      title = message(locale, 'about');
      content = (
        <div className={dialogStackStyle}>
          <div>{localized(adapter.product.description, locale)}</div>
          <div>
            {`${message(locale, 'aboutEngine')}: ${adapter.product.engineVersion}`}
          </div>
          <Link href={adapter.product.repositoryUrl} target="_blank">
            {message(locale, 'aboutRepository')}
          </Link>
        </div>
      );
      break;
    case 'commands':
      title = message(locale, 'commandPalette');
      content = (
        <CommandContent
          adapter={adapter}
          locale={locale}
          onRun={onRunCommand}
        />
      );
      break;
    case 'outline':
      title = message(locale, 'outline');
      content = (
        <OutlineList
          entries={analysis.outline}
          locale={locale}
          onJump={onJump}
        />
      );
      break;
    case 'diagnostics':
      title = `${message(locale, 'diagnostics')} (${analysis.diagnostics.length})`;
      content = (
        <DiagnosticsList
          diagnostics={analysis.diagnostics}
          locale={locale}
          onJump={onJump}
        />
      );
      break;
    default:
      break;
  }

  return (
    <DialogContainer onDismiss={onDismiss}>
      {dialog && (
        <Dialog isDismissible size="L">
          <Heading slot="title">{title}</Heading>
          <Content>{content}</Content>
        </Dialog>
      )}
    </DialogContainer>
  );
}

function DirectionControl({
  direction,
  locale,
  onChange,
}: {
  readonly direction: WritingDirection;
  readonly locale: Locale;
  readonly onChange: (direction: WritingDirection) => void;
}) {
  return (
    <SegmentedControl
      aria-label={message(locale, 'direction')}
      onSelectionChange={(key) => onChange(String(key) as WritingDirection)}
      selectedKey={direction}
    >
      <SegmentedControlItem id="horizontal">
        {message(locale, 'writingHorizontal')}
      </SegmentedControlItem>
      <SegmentedControlItem id="vertical">
        {message(locale, 'writingVertical')}
      </SegmentedControlItem>
    </SegmentedControl>
  );
}

function PreviewPane({
  adapter,
  analysis,
  analysisError,
  diagnosticsExpanded,
  direction,
  locale,
  onDiagnosticsExpandedChange,
  onDirectionChange,
  onJump,
  showDiagnostics,
}: {
  readonly adapter: PlaygroundAdapter;
  readonly analysis: PlaygroundAnalysis;
  readonly analysisError: boolean;
  readonly diagnosticsExpanded: boolean;
  readonly direction: WritingDirection;
  readonly locale: Locale;
  readonly onDiagnosticsExpandedChange: (expanded: boolean) => void;
  readonly onDirectionChange: (direction: WritingDirection) => void;
  readonly onJump: (range: TextRange) => void;
  readonly showDiagnostics: boolean;
}) {
  return (
    <section className={paneStyle} aria-label={message(locale, 'preview')}>
      <div className={paneHeaderStyle}>
        <span className={paneTitleStyle}>{message(locale, 'preview')}</span>
        <DirectionControl
          direction={direction}
          locale={locale}
          onChange={onDirectionChange}
        />
      </div>
      {analysisError && (
        <InlineAlert variant="negative">
          <Heading>{message(locale, 'analyzeFailed')}</Heading>
        </InlineAlert>
      )}
      <div className={previewHostStyle}>
        <PreviewMount
          adapter={adapter}
          direction={direction}
          html={analysis.html}
        />
      </div>
      {showDiagnostics && (
        <DiagnosticsDisclosure
          diagnostics={analysis.diagnostics}
          isExpanded={diagnosticsExpanded}
          locale={locale}
          onExpandedChange={onDiagnosticsExpandedChange}
          onJump={onJump}
        />
      )}
    </section>
  );
}

function EditorPane({
  adapter,
  locale,
  onChange,
  onController,
  settingValues,
  value,
}: {
  readonly adapter: PlaygroundAdapter;
  readonly locale: Locale;
  readonly onChange: (value: string) => void;
  readonly onController: (controller: EditorController | null) => void;
  readonly settingValues: Readonly<Record<string, boolean>>;
  readonly value: string;
}) {
  return (
    <section
      className={`${paneStyle} ${editorBorderStyle}`}
      aria-label={message(locale, 'editor')}
    >
      <div className={paneHeaderStyle}>
        <span className={paneTitleStyle}>{message(locale, 'editor')}</span>
      </div>
      <EditorMount
        adapter={adapter}
        locale={locale}
        onChange={onChange}
        onController={onController}
        settingValues={settingValues}
        value={value}
      />
    </section>
  );
}

function LayoutControl({
  layout,
  locale,
  onChange,
}: {
  readonly layout: LayoutMode;
  readonly locale: Locale;
  readonly onChange: (layout: LayoutMode) => void;
}) {
  return (
    <SegmentedControl
      aria-label={message(locale, 'layout')}
      onSelectionChange={(key) => onChange(String(key) as LayoutMode)}
      selectedKey={layout}
    >
      <SegmentedControlItem id="editor">
        {message(locale, 'layoutEditor')}
      </SegmentedControlItem>
      <SegmentedControlItem id="split">
        {message(locale, 'layoutSplit')}
      </SegmentedControlItem>
      <SegmentedControlItem id="preview">
        {message(locale, 'layoutPreview')}
      </SegmentedControlItem>
    </SegmentedControl>
  );
}

export function PlaygroundApp({ adapter }: PlaygroundAppProps) {
  const boot = useMemo(() => initialSource(adapter), [adapter]);
  const [source, setSource] = useState(boot.source);
  const [preferences, setPreferences] = useState(loadPreferences);
  const [settingValues, setSettingValues] = useState(() =>
    loadSettingValues(adapter.product.id, adapter.settings),
  );
  const [initialization, setInitialization] =
    useState<InitializationState>('loading');
  const [analysis, setAnalysis] = useState<PlaygroundAnalysis>(EMPTY_ANALYSIS);
  const [analysisError, setAnalysisError] = useState(false);
  const [diagnosticsExpanded, setDiagnosticsExpanded] = useState(false);
  const [dialog, setDialog] = useState<DialogName>(null);
  const [selectedSample, setSelectedSample] = useState<string | null>(null);
  const [mobilePane, setMobilePane] = useState<'editor' | 'preview'>('editor');
  const [editorController, setEditorController] =
    useState<EditorController | null>(null);
  const [pendingJump, setPendingJump] = useState<TextRange | null>(null);
  const [pendingCommand, setPendingCommand] = useState<string | null>(null);
  const isMobile = useMedia('(max-width: 767px)');
  const isCompact = useMedia('(max-width: 1100px)');
  const colorScheme = useEffectiveColorScheme(preferences.colorScheme);
  const revisionRef = useRef(0);
  const initializationRevisionRef = useRef(0);
  const previousAttentionDiagnostics = useRef<Set<string> | null>(null);
  const storageFailureShownRef = useRef(false);

  const locale = preferences.locale;
  const setPreference = useCallback(
    <Key extends keyof UserPreferences>(
      key: Key,
      value: UserPreferences[Key],
    ) => {
      setPreferences((current) => ({ ...current, [key]: value }));
    },
    [],
  );

  const onEditorChange = useCallback((value: string) => {
    setSelectedSample(null);
    setSource(value);
  }, []);

  const onEditorController = useCallback(
    (controller: EditorController | null) => {
      setEditorController(controller);
    },
    [],
  );

  const reportStorageResult = useCallback(
    (saved: boolean) => {
      if (saved) {
        storageFailureShownRef.current = false;
      } else if (!storageFailureShownRef.current) {
        storageFailureShownRef.current = true;
        ToastQueue.negative(message(locale, 'storageFailed'));
      }
    },
    [locale],
  );

  useEffect(() => {
    document.documentElement.lang = locale;
    document.documentElement.dataset.colorScheme = colorScheme;
    document.title = `${adapter.product.name} — ${localized(
      adapter.product.description,
      locale,
    )}`;
    adapter.setLocale?.(locale);
    reportStorageResult(savePreferences(preferences));
  }, [adapter, colorScheme, locale, preferences, reportStorageResult]);

  useEffect(() => {
    const pending = globalThis.setTimeout(() => {
      reportStorageResult(saveDraft(adapter.product.id, source));
    }, 300);
    return () => globalThis.clearTimeout(pending);
  }, [adapter.product.id, reportStorageResult, source]);

  const initialize = useCallback(() => {
    const revision = ++initializationRevisionRef.current;
    setInitialization('loading');
    adapter.initialize().then(
      () => {
        if (revision === initializationRevisionRef.current) {
          setInitialization('ready');
        }
      },
      () => {
        if (revision === initializationRevisionRef.current) {
          setInitialization('failed');
        }
      },
    );
  }, [adapter]);

  useEffect(() => {
    if (!adapter.createEditorDuringInitialization) {
      initialize();
      return;
    }
    let secondFrame = 0;
    let timer = 0;
    const firstFrame = globalThis.requestAnimationFrame(() => {
      secondFrame = globalThis.requestAnimationFrame(() => {
        timer = globalThis.setTimeout(
          initialize,
          POST_PAINT_WORK_DELAY_MS,
        );
      });
    });
    return () => {
      globalThis.cancelAnimationFrame(firstFrame);
      globalThis.cancelAnimationFrame(secondFrame);
      globalThis.clearTimeout(timer);
    };
  }, [adapter.createEditorDuringInitialization, initialize]);

  useEffect(() => {
    if (
      adapter.createEditorDuringInitialization &&
      initialization === 'failed' &&
      isMobile
    ) {
      setMobilePane('preview');
    }
  }, [
    adapter.createEditorDuringInitialization,
    initialization,
    isMobile,
  ]);

  useEffect(() => {
    if (initialization !== 'ready') return;
    const revision = ++revisionRef.current;
    const abort = new AbortController();
    const pending = globalThis.setTimeout(() => {
      adapter
        .analyze(source, { revision, signal: abort.signal })
        .then((result) => {
          if (abort.signal.aborted || revision !== revisionRef.current) return;
          setAnalysis(result);
          setAnalysisError(false);
          const attentionDiagnostics = new Set(
            result.diagnostics
              .filter((diagnostic) => diagnostic.severity !== 'info')
              .map(
                (diagnostic) =>
                  `${diagnostic.severity}:${diagnostic.code ?? ''}:${diagnostic.range.start}:${diagnostic.range.end}:${diagnostic.message.ja}:${diagnostic.message.en}`,
              ),
          );
          const previous = previousAttentionDiagnostics.current;
          if (
            previous !== null &&
            [...attentionDiagnostics].some((key) => !previous.has(key))
          ) {
            setDiagnosticsExpanded(true);
          }
          previousAttentionDiagnostics.current = attentionDiagnostics;
        })
        .catch(() => {
          if (abort.signal.aborted || revision !== revisionRef.current) return;
          setAnalysisError(true);
        });
    }, 100);
    return () => {
      globalThis.clearTimeout(pending);
      abort.abort();
    };
  }, [adapter, initialization, locale, source]);

  useEffect(() => {
    if (!boot.invalidUrl) return;
    ToastQueue.negative(message(locale, 'urlInvalid'));
  }, [boot.invalidUrl, locale]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (
        event.key.toLocaleLowerCase() === 'p' &&
        event.shiftKey &&
        (event.ctrlKey || event.metaKey)
      ) {
        event.preventDefault();
        setDialog('commands');
      }
    };
    globalThis.addEventListener('keydown', onKeyDown);
    return () => globalThis.removeEventListener('keydown', onKeyDown);
  }, []);

  useEffect(() => {
    if (!pendingJump || !editorController) return;
    return afterDialogsClose(() => {
      editorController.revealRange(pendingJump);
      editorController.focus();
      setPendingJump(null);
    });
  }, [editorController, pendingJump]);

  useEffect(() => {
    if (!pendingCommand || !editorController) return;
    const command = pendingCommand;
    return afterDialogsClose(() => {
      editorController.runCommand(command);
      editorController.focus();
      setPendingCommand(null);
    });
  }, [editorController, pendingCommand]);

  const setSetting = useCallback(
    (id: string, value: boolean) => {
      setSettingValues((current) => {
        const next = { ...current, [id]: value };
        reportStorageResult(saveSettingValues(adapter.product.id, next));
        return next;
      });
      editorController?.setSetting(id, value);
    },
    [adapter.product.id, editorController, reportStorageResult],
  );

  const jumpToRange = useCallback(
    (range: TextRange) => {
      setDialog(null);
      if (isMobile) {
        setMobilePane('editor');
        setPendingJump(range);
        return;
      }
      if (editorController) {
        editorController.revealRange(range);
        editorController.focus();
      } else {
        setPreference('layout', 'split');
        setPendingJump(range);
      }
    },
    [editorController, isMobile, setPreference],
  );

  const runCommand = useCallback(
    (id: string) => {
      setDialog(null);
      if (isMobile) setMobilePane('editor');
      if (!editorController && !isMobile) setPreference('layout', 'split');
      setPendingCommand(id);
    },
    [editorController, isMobile, setPreference],
  );

  const loadSample = useCallback(
    (id: string) => {
      const sample = adapter.samples.find((candidate) => candidate.id === id);
      if (!sample) return;
      setSelectedSample(id);
      setSource(sample.source);
      ToastQueue.positive(message(locale, 'sampleLoaded'));
    },
    [adapter.samples, locale],
  );

  const share = useCallback(() => {
    copyShareUrl(source).then(
      () => ToastQueue.positive(message(locale, 'shareCopied')),
      (error: unknown) =>
        ToastQueue.negative(
          message(
            locale,
            error instanceof ShareUrlTooLongError
              ? 'shareTooLong'
              : 'shareFailed',
          ),
        ),
    );
  }, [locale, source]);

  const menuAction = useCallback(
    (key: React.Key) => {
      const action = String(key);
      if (action.startsWith('sample:')) {
        loadSample(action.slice('sample:'.length));
        return;
      }
      if (action.startsWith('layout:')) {
        setPreference('layout', action.slice('layout:'.length) as LayoutMode);
        return;
      }
      switch (action) {
        case 'commands':
        case 'guide':
        case 'settings':
        case 'about':
          setDialog(action);
          break;
        case 'outline':
          setPreference('outlineOpen', !preferences.outlineOpen);
          break;
        case 'share':
          share();
          break;
        default:
          break;
      }
    },
    [loadSample, preferences.outlineOpen, setPreference, share],
  );

  const headerMenu = (
    <ActionMenu
      aria-label={message(locale, 'more')}
      onAction={menuAction}
      size="M"
    >
      {adapter.samples.map((sample) => (
        <MenuItem id={`sample:${sample.id}`} key={sample.id}>
          <TextIcon />
          <MenuText>{localized(sample.title, locale)}</MenuText>
        </MenuItem>
      ))}
      {!isMobile && (
        <MenuItem id="layout:editor">
          <TextIcon />
          <MenuText>{message(locale, 'layoutEditor')}</MenuText>
        </MenuItem>
      )}
      {!isMobile && (
        <MenuItem id="layout:split">
          <TextIcon />
          <MenuText>{message(locale, 'layoutSplit')}</MenuText>
        </MenuItem>
      )}
      {!isMobile && (
        <MenuItem id="layout:preview">
          <Preview />
          <MenuText>{message(locale, 'layoutPreview')}</MenuText>
        </MenuItem>
      )}
      {!isMobile && (
        <MenuItem id="outline">
          <ViewList />
          <MenuText>{message(locale, 'outline')}</MenuText>
        </MenuItem>
      )}
      <MenuItem id="commands">
        <Code />
        <MenuText>{message(locale, 'commands')}</MenuText>
      </MenuItem>
      <MenuItem id="guide">
        <HelpCircle />
        <MenuText>{message(locale, 'guide')}</MenuText>
      </MenuItem>
      <MenuItem id="share">
        <Share />
        <MenuText>{message(locale, 'share')}</MenuText>
      </MenuItem>
      <MenuItem id="settings">
        <Settings />
        <MenuText>{message(locale, 'settings')}</MenuText>
      </MenuItem>
      <MenuItem id="about">
        <InfoCircle />
        <MenuText>{message(locale, 'about')}</MenuText>
      </MenuItem>
    </ActionMenu>
  );

  const editorPane = (
    <EditorPane
      adapter={adapter}
      locale={locale}
      onChange={onEditorChange}
      onController={onEditorController}
      settingValues={settingValues}
      value={source}
    />
  );
  const previewPane =
    initialization === 'ready' ? (
      <PreviewPane
        adapter={adapter}
        analysis={analysis}
        analysisError={analysisError}
        diagnosticsExpanded={diagnosticsExpanded}
        direction={preferences.writingDirection}
        locale={locale}
        onDiagnosticsExpandedChange={setDiagnosticsExpanded}
        onDirectionChange={(direction) =>
          setPreference('writingDirection', direction)
        }
        onJump={jumpToRange}
        showDiagnostics={!isMobile}
      />
    ) : (
      <section
        aria-label={message(locale, 'preview')}
        className={paneStyle}
      >
        <div className={paneHeaderStyle}>
          <span className={paneTitleStyle}>{message(locale, 'preview')}</span>
        </div>
        {initialization === 'loading' ? (
          <div className={centeredStyle} role="status">
            <ProgressCircle
              aria-label={message(locale, 'initializing')}
              isIndeterminate
            />
            <span>{message(locale, 'initializing')}</span>
          </div>
        ) : (
          <div className={centeredStyle}>
            <InlineAlert autoFocus variant="negative">
              <Heading>{message(locale, 'initializationFailed')}</Heading>
              <Content>{message(locale, 'initializationHint')}</Content>
            </InlineAlert>
            <Button onPress={initialize} variant="accent">
              {message(locale, 'retry')}
            </Button>
          </div>
        )}
      </section>
    );

  let workspace: ReactNode;
  if (
    initialization === 'loading' &&
    !adapter.createEditorDuringInitialization
  ) {
    workspace = (
      <div className={centeredStyle} role="status">
        <ProgressCircle
          aria-label={message(locale, 'initializing')}
          isIndeterminate
        />
        <span>{message(locale, 'initializing')}</span>
      </div>
    );
  } else if (
    initialization === 'failed' &&
    !adapter.createEditorDuringInitialization
  ) {
    workspace = (
      <div className={centeredStyle}>
        <InlineAlert autoFocus variant="negative">
          <Heading>{message(locale, 'initializationFailed')}</Heading>
          <Content>{message(locale, 'initializationHint')}</Content>
        </InlineAlert>
        <Button onPress={initialize} variant="accent">
          {message(locale, 'retry')}
        </Button>
      </div>
    );
  } else if (isMobile) {
    workspace = (
      <div className={paneStyle}>
        <div className={mobileToolsStyle}>
          <ActionButton
            isDisabled={initialization !== 'ready'}
            onPress={() => setDialog('outline')}
            size="S"
          >
            <ViewList />
            <ActionButtonText>{message(locale, 'outline')}</ActionButtonText>
          </ActionButton>
          <ActionButton
            isDisabled={initialization !== 'ready'}
            onPress={() => setDialog('diagnostics')}
            size="S"
          >
            <InfoCircle />
            <ActionButtonText>
              {`${message(locale, 'diagnostics')} (${analysis.diagnostics.length})`}
            </ActionButtonText>
          </ActionButton>
        </div>
        <Tabs
          aria-label={message(locale, 'layout')}
          onSelectionChange={(key) =>
            setMobilePane(String(key) as 'editor' | 'preview')
          }
          selectedKey={mobilePane}
          styles={mobileTabsStyle}
        >
          <TabList>
            <Tab id="editor">
              <TextIcon />
              <ActionButtonText>{message(locale, 'editor')}</ActionButtonText>
            </Tab>
            <Tab id="preview">
              <Preview />
              <ActionButtonText>{message(locale, 'preview')}</ActionButtonText>
            </Tab>
          </TabList>
          <TabPanel id="editor" styles={mobilePanelStyle}>
            <div className={mobilePanelContentStyle}>{editorPane}</div>
          </TabPanel>
          <TabPanel id="preview" styles={mobilePanelStyle}>
            <div className={mobilePanelContentStyle}>{previewPane}</div>
          </TabPanel>
        </Tabs>
      </div>
    );
  } else {
    const forceSplit = initialization !== 'ready';
    workspace = (
      <div className={workspaceStyle}>
        {initialization === 'ready' && preferences.outlineOpen && (
          <aside
            className={outlineStyle}
            aria-label={message(locale, 'outline')}
          >
            <div className={paneHeaderStyle}>
              <span className={paneTitleStyle}>
                {message(locale, 'outline')}
              </span>
            </div>
            <OutlineList
              entries={analysis.outline}
              locale={locale}
              onJump={jumpToRange}
            />
          </aside>
        )}
        <div
          className={`${panesStyle} ${
            forceSplit || preferences.layout === 'split'
              ? splitPanesStyle
              : singlePaneStyle
          }`}
        >
          {(forceSplit || preferences.layout !== 'preview') && editorPane}
          {(forceSplit || preferences.layout !== 'editor') && previewPane}
        </div>
      </div>
    );
  }

  return (
    <Provider
      background="base"
      colorScheme={colorScheme}
      locale={locale === 'ja' ? 'ja-JP' : 'en-US'}
      styles={shellStyle}
    >
      <header className={headerStyle}>
        <div className={brandStyle}>
          <h1 className={brandNameStyle}>{adapter.product.name}</h1>
          {!isCompact && (
            <span className={brandDescriptionStyle}>
              {localized(adapter.product.description, locale)}
            </span>
          )}
        </div>
        <div className={headerActionsStyle}>
          {isCompact ? (
            headerMenu
          ) : (
            <>
              <Picker
                aria-label={message(locale, 'sample')}
                onSelectionChange={(key) => loadSample(String(key))}
                placeholder={message(locale, 'sample')}
                selectedKey={selectedSample}
                styles={style({ width: 240 })}
              >
                {adapter.samples.map((sample) => (
                  <PickerItem id={sample.id} key={sample.id}>
                    {localized(sample.title, locale)}
                  </PickerItem>
                ))}
              </Picker>
              {!isMobile && (
                <>
                  <LayoutControl
                    layout={preferences.layout}
                    locale={locale}
                    onChange={(layout) => setPreference('layout', layout)}
                  />
                  <ToggleButton
                    aria-label={message(locale, 'outline')}
                    isSelected={preferences.outlineOpen}
                    onChange={(selected) =>
                      setPreference('outlineOpen', selected)
                    }
                  >
                    <ViewList />
                  </ToggleButton>
                </>
              )}
              <ActionButton onPress={() => setDialog('commands')}>
                <Code />
                <ActionButtonText>
                  {message(locale, 'commands')}
                </ActionButtonText>
              </ActionButton>
              <ActionButton onPress={() => setDialog('guide')}>
                <HelpCircle />
                <ActionButtonText>{message(locale, 'guide')}</ActionButtonText>
              </ActionButton>
              <ActionButton onPress={share}>
                <Share />
                <ActionButtonText>{message(locale, 'share')}</ActionButtonText>
              </ActionButton>
              <ActionButton
                aria-label={message(locale, 'settings')}
                onPress={() => setDialog('settings')}
              >
                <Settings />
              </ActionButton>
              <ActionButton
                aria-label={message(locale, 'about')}
                onPress={() => setDialog('about')}
              >
                <InfoCircle />
              </ActionButton>
            </>
          )}
        </div>
      </header>
      <main className={workspaceStyle}>{workspace}</main>
      <ActiveDialog
        adapter={adapter}
        analysis={analysis}
        dialog={dialog}
        locale={locale}
        onDismiss={() => setDialog(null)}
        onJump={jumpToRange}
        onRunCommand={runCommand}
        preferences={preferences}
        setPreference={setPreference}
        settingValues={settingValues}
        setSetting={setSetting}
      />
      <ToastContainer placement="bottom end" />
    </Provider>
  );
}
