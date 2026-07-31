export type Locale = 'ja' | 'en';
export type ColorSchemePreference = 'auto' | 'light' | 'dark';
export type LayoutMode = 'editor' | 'split' | 'preview';
export type WritingDirection = 'horizontal' | 'vertical';
export type DiagnosticSeverity = 'info' | 'warning' | 'error';

export interface LocalizedText {
  readonly ja: string;
  readonly en: string;
}

export interface TextRange {
  readonly start: number;
  readonly end: number;
}

export interface PlaygroundDiagnostic {
  readonly severity: DiagnosticSeverity;
  readonly message: LocalizedText;
  readonly range: TextRange;
  readonly code?: string;
}

export interface PlaygroundOutlineEntry {
  readonly level: number;
  readonly text: string;
  readonly range: TextRange | null;
}

export interface PlaygroundAnalysis {
  readonly html: string;
  readonly diagnostics: readonly PlaygroundDiagnostic[];
  readonly outline: readonly PlaygroundOutlineEntry[];
}

export interface PlaygroundSample {
  readonly id: string;
  readonly title: LocalizedText;
  readonly source: string;
}

export interface PlaygroundCommand {
  readonly id: string;
  readonly label: LocalizedText;
  readonly shortcut?: string;
}

export interface PlaygroundSetting {
  readonly id: string;
  readonly label: LocalizedText;
  readonly description: LocalizedText;
  readonly defaultValue: boolean;
}

export interface PlaygroundGuideSection {
  readonly id: string;
  readonly title: LocalizedText;
  readonly body: LocalizedText;
  readonly example?: string;
  readonly href?: string;
}

export interface PlaygroundGuide {
  readonly title: LocalizedText;
  readonly introduction: LocalizedText;
  readonly sections: readonly PlaygroundGuideSection[];
}

export interface PlaygroundProduct {
  readonly id: string;
  readonly name: string;
  readonly shortName: string;
  readonly description: LocalizedText;
  readonly repositoryUrl: string;
  readonly engineVersion: string;
}

export interface EditorController {
  setValue(value: string): void;
  focus(): void;
  revealRange(range: TextRange): void;
  runCommand(commandId: string): boolean;
  setSetting(settingId: string, value: boolean): void;
  destroy(): void;
}

export interface PreviewController {
  update(html: string, direction: WritingDirection): void;
  destroy(): void;
}

export interface AnalyzeContext {
  readonly revision: number;
  readonly signal: AbortSignal;
}

export interface PlaygroundAdapter {
  readonly product: PlaygroundProduct;
  readonly samples: readonly PlaygroundSample[];
  readonly guide: PlaygroundGuide;
  readonly commands: readonly PlaygroundCommand[];
  readonly settings: readonly PlaygroundSetting[];
  /** The adapter can mount a basic editor while its rendering engine initializes. */
  readonly createEditorDuringInitialization?: boolean;
  setLocale?(locale: Locale): void;
  initialize(): Promise<void>;
  analyze(source: string, context: AnalyzeContext): Promise<PlaygroundAnalysis>;
  createEditor(
    parent: HTMLElement,
    initialValue: string,
    onChange: (value: string) => void,
  ): EditorController | Promise<EditorController>;
  createPreview(parent: HTMLElement): PreviewController;
}

export interface PlaygroundAppProps {
  readonly adapter: PlaygroundAdapter;
}
