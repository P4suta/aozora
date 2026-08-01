export const WORKSPACE_LINT_FILE_LIMIT = 50_000;
export const WORKSPACE_LINT_SOURCE_LIMIT = 16 * 1024 * 1024;
export const AOZORA_WIRE_SCHEMA_VERSION = 3;

export function workspaceLintSourceSizeSupported(size: number): boolean {
  return Number.isSafeInteger(size) && size >= 0 && size <= WORKSPACE_LINT_SOURCE_LIMIT;
}

export interface LintProcessResult {
  readonly code: number;
  readonly stdout: string;
  readonly stderr: string;
}

export interface FormattedLintResult {
  readonly lines: readonly string[];
  readonly failed: boolean;
}

interface JsonShape {
  readonly data?: unknown;
  readonly kind?: unknown;
  readonly schemaVersion?: unknown;
  readonly severity?: unknown;
  readonly source?: unknown;
  readonly span?: unknown;
  readonly end?: unknown;
  readonly start?: unknown;
}

function jsonShape(value: unknown): JsonShape | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as JsonShape)
    : undefined;
}

function sourceSpanPosition(
  source: string,
  startByte: number,
  endByte: number,
): { line: number; column: number } {
  let byte = 0;
  let line = 1;
  let column = 1;
  let startPosition: { line: number; column: number } | undefined;
  for (const character of source) {
    if (byte === startByte) {
      startPosition = { line, column };
    }
    if (byte === endByte) {
      if (!startPosition) {
        throw new Error("invalid diagnostic span");
      }
      return startPosition;
    }
    const nextByte = byte + Buffer.byteLength(character, "utf8");
    if ((startByte > byte && startByte < nextByte) || (endByte > byte && endByte < nextByte)) {
      throw new Error("diagnostic span is not on a UTF-8 boundary");
    }
    const leadingBom = byte === 0 && character === "\u{feff}";
    byte = nextByte;
    if (leadingBom) {
      continue;
    }
    if (character === "\n") {
      line++;
      column = 1;
    } else {
      column += character.length;
    }
  }
  if (byte === startByte) {
    startPosition = { line, column };
  }
  if (byte === endByte && startPosition) {
    return startPosition;
  }
  throw new Error("diagnostic span exceeds the source");
}

function diagnosticLine(value: unknown, file: string, source: string): string {
  const diagnostic = jsonShape(value);
  const span = jsonShape(diagnostic?.span);
  if (
    !diagnostic ||
    typeof diagnostic.kind !== "string" ||
    diagnostic.kind.length === 0 ||
    (diagnostic.source !== "source" && diagnostic.source !== "internal") ||
    !span
  ) {
    throw new Error("invalid diagnostic");
  }
  const kind = diagnostic.kind;
  const start = span.start;
  const end = span.end;
  if (
    typeof start !== "number" ||
    !Number.isSafeInteger(start) ||
    start < 0 ||
    typeof end !== "number" ||
    !Number.isSafeInteger(end) ||
    end < start
  ) {
    throw new Error("invalid diagnostic span");
  }
  if (
    diagnostic.severity !== "error" &&
    diagnostic.severity !== "warning" &&
    diagnostic.severity !== "note"
  ) {
    throw new Error("invalid diagnostic severity");
  }
  const severity = diagnostic.severity === "note" ? "info" : diagnostic.severity;
  const { line, column } = sourceSpanPosition(source, start, end);
  const message = kind.replaceAll("_", " ");
  return `${file}:${line}:${column}: ${severity}[${kind}]: ${message}`;
}

export function formatLintJson(output: string, file: string, source: string): string[] {
  const envelope = jsonShape(JSON.parse(output));
  if (envelope?.schemaVersion !== AOZORA_WIRE_SCHEMA_VERSION || !Array.isArray(envelope.data)) {
    throw new Error("invalid aozora JSON envelope");
  }
  return envelope.data.map((diagnostic) => diagnosticLine(diagnostic, file, source));
}

export function formatLintProcessResult(
  result: LintProcessResult,
  file: string,
  source: string,
): FormattedLintResult {
  if (result.stdout.trim().length > 0) {
    throw new Error("aozora lint wrote unexpected output to stdout");
  }
  if (result.stderr.trim().length === 0) {
    if (result.code !== 0) {
      throw new Error(`aozora lint exited with status ${result.code} without diagnostics`);
    }
    return { lines: [], failed: false };
  }
  try {
    return {
      lines: formatLintJson(result.stderr, file, source),
      failed: result.code !== 0,
    };
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    if (result.code === 0) {
      throw new Error(`aozora lint emitted invalid diagnostic JSON on stderr: ${detail}`);
    }
    throw new Error(`aozora lint exited with status ${result.code}: ${result.stderr.trim()}`);
  }
}

export function decodeLintSource(bytes: Uint8Array): string {
  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    return new TextDecoder("shift_jis", { fatal: true }).decode(bytes);
  }
}
