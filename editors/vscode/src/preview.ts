import { basename } from "node:path";

import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";
import { AsyncGeneration } from "./extensionLogic";
import { parseRenderHtmlResult } from "./lspWire";
import { aozoraNotationStyles } from "./notationStyles";

type WritingMode = "vertical" | "horizontal";

interface PreviewState {
  panel: vscode.WebviewPanel;
  uri: vscode.Uri;
  mode: WritingMode;
  generation: AsyncGeneration;
  disposed: boolean;
  debounce?: NodeJS.Timeout;
}

const PREVIEW_RENDER_DEBOUNCE_MS = 150;

const panelsByUri: Map<string, PreviewState> = new Map();

function configuredWritingMode(): WritingMode {
  const raw = vscode.workspace
    .getConfiguration("aozora.preview")
    .get<string>("writingMode", "vertical");
  return raw === "horizontal" ? "horizontal" : "vertical";
}

export function registerPreviewCommand(
  context: vscode.ExtensionContext,
  client: LanguageClient,
): void {
  context.subscriptions.push(
    vscode.commands.registerCommand("aozora.openPreview", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        void vscode.window.showInformationMessage(
          "Open an aozora document first, then run this command.",
        );
        return;
      }
      await openPreview(editor.document, client, context);
    }),
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("aozora.preview.toggleWritingMode", async () => {
      const target = pickPanelForToggle();
      if (!target) {
        void vscode.window.showInformationMessage(
          "プレビューが開かれていません。`Aozora: Open Preview` で開いてから実行してください。",
        );
        return;
      }
      target.mode = target.mode === "vertical" ? "horizontal" : "vertical";
      await renderInto(target, client);
    }),
  );

  context.subscriptions.push(
    vscode.workspace.onDidChangeTextDocument((event) => {
      const state = panelsByUri.get(event.document.uri.toString());
      if (!state) {
        return;
      }
      state.generation.invalidate();
      if (state.debounce) {
        clearTimeout(state.debounce);
      }
      state.debounce = setTimeout(() => {
        void renderInto(state, client);
      }, PREVIEW_RENDER_DEBOUNCE_MS);
    }),
  );

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (!event.affectsConfiguration("aozora.preview.writingMode")) {
        return;
      }
      const next = configuredWritingMode();
      for (const state of panelsByUri.values()) {
        state.mode = next;
        void renderInto(state, client);
      }
    }),
  );
}

function pickPanelForToggle(): PreviewState | undefined {
  const activeUri = vscode.window.activeTextEditor?.document.uri.toString();
  if (activeUri !== undefined) {
    const fromActive = panelsByUri.get(activeUri);
    if (fromActive) {
      return fromActive;
    }
  }
  if (panelsByUri.size === 1) {
    return panelsByUri.values().next().value;
  }
  return undefined;
}

async function openPreview(
  document: vscode.TextDocument,
  client: LanguageClient,
  context: vscode.ExtensionContext,
): Promise<void> {
  const key = document.uri.toString();
  const existing = panelsByUri.get(key);
  if (existing) {
    existing.panel.reveal(vscode.ViewColumn.Beside);
    await renderInto(existing, client);
    return;
  }

  const panel = vscode.window.createWebviewPanel(
    "aozoraPreview",
    `Aozora Preview — ${basename(document.fileName) || "untitled"}`,
    vscode.ViewColumn.Beside,
    {
      enableScripts: false,
      localResourceRoots: [],
      retainContextWhenHidden: true,
    },
  );

  const state: PreviewState = {
    panel,
    uri: document.uri,
    mode: configuredWritingMode(),
    generation: new AsyncGeneration(),
    disposed: false,
  };
  panelsByUri.set(key, state);

  panel.onDidDispose(
    () => {
      const live = panelsByUri.get(key);
      state.disposed = true;
      state.generation.dispose();
      if (state.debounce) {
        clearTimeout(state.debounce);
      }
      if (live === state) {
        panelsByUri.delete(key);
      }
    },
    undefined,
    context.subscriptions,
  );

  await renderInto(state, client);
}

async function renderInto(state: PreviewState, client: LanguageClient): Promise<void> {
  if (state.disposed) {
    return;
  }
  const generation = state.generation.begin();
  try {
    const response = await client.sendRequest<unknown>("aozora/renderHtml", {
      uri: state.uri.toString(),
    });
    if (!state.generation.isCurrent(generation)) {
      return;
    }
    const result = parseRenderHtmlResult(response);
    state.panel.webview.html = wrapHtml(result.html, state.mode);
  } catch (err) {
    if (!state.generation.isCurrent(generation)) {
      return;
    }
    const message = err instanceof Error ? err.message : String(err);
    try {
      state.panel.webview.html = wrapHtml(
        `<pre>aozora/renderHtml failed: ${escapeHtml(message)}</pre>`,
        state.mode,
      );
    } catch (writeError) {
      if (state.disposed) {
        return;
      }
      console.error(
        `Failed to write Aozora preview error page for ${state.uri.toString()}`,
        writeError,
      );
    }
  }
}

function wrapHtml(body: string, mode: WritingMode): string {
  const verticalChrome =
    mode === "vertical"
      ? `
    body {
      max-width: none;
      max-height: none;
      height: calc(100vh - 3em);
      margin: 1.5em 0;
      padding: 1em 2em;
    }
  `
      : "";
  const bodyClass = mode === "vertical" ? "aozora-notation aozora-vertical" : "aozora-notation";
  return `<!DOCTYPE html>
<html lang="ja">
<head>
  <meta charset="utf-8" />
  <meta http-equiv="Content-Security-Policy"
    content="default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'" />
  <title>Aozora Preview</title>
  <style>
    body {
      max-width: 42em;
      margin: 1.5em auto;
      padding: 0 1em;
      background: var(--vscode-editor-background, #fdf6e3);
    }
    h1, h2, h3 {
      font-weight: 600;
      letter-spacing: 0.05em;
      margin-top: 2em;
    }
    p { margin: 0.5em 0; }
    ${aozoraNotationStyles}
    ${verticalChrome}
  </style>
</head>
<body class="${bodyClass}">
${body}
</body>
</html>`;
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
