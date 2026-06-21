export interface Span {
  start: number;
  end: number;
}

export interface DiagnosticEntry {
  kind: string;
  span: Span;
  codepoint?: number;
}

export interface NodeEntry {
  kind: string;
  span: Span;
}

export interface JsonEnvelope<T> {
  schemaVersion: number;
  data: T[];
}
