// aozora preview pane.
//
// `aozora.openPreview` opens (or focuses) a side-by-side WebviewPanel
// that shows the rendered HTML output for the active aozora document.
// Re-renders on `onDidChangeTextDocument` (debounced ~150 ms) by
// asking the LSP server through the custom `aozora/renderHtml`
// request.
//
// Each panel carries its own writing-mode (縦書き / 横書き). The
// initial value comes from the `aozora.preview.writingMode`
// configuration; `aozora.preview.toggleWritingMode` flips the active
// panel without touching the setting (so the user's per-session
// preference doesn't overwrite the workspace default). The default
// is 縦書き — Aozora Bunko works are vertically typeset in print, so
// the preview matches that orientation by default.

import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

import { aozoraNotationStyles } from "./notationStyles";

interface RenderHtmlResult {
  html: string;
}

type WritingMode = "vertical" | "horizontal";

/**
 * Hidden state per WebviewPanel — tracks the document URI it follows,
 * the active writing mode, and a debounce handle so rapid edits
 * coalesce into a single render.
 */
interface PreviewState {
  panel: vscode.WebviewPanel;
  uri: vscode.Uri;
  mode: WritingMode;
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
      if (state.debounce) {
        clearTimeout(state.debounce);
      }
      state.debounce = setTimeout(() => {
        void renderInto(state, client);
      }, PREVIEW_RENDER_DEBOUNCE_MS);
    }),
  );

  // When the workspace `aozora.preview.writingMode` setting changes,
  // re-apply it to every open panel that hasn't been overridden by
  // the toggle command since opening. Detecting "hasn't been
  // overridden" is fiddly without extra state, so we apply the new
  // setting unconditionally here — the toggle command's effect is
  // explicitly documented as "until you change the setting or close
  // the panel".
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

/**
 * Resolve which panel the toggle command should act on.
 *
 * Order:
 *   1. The panel whose document is currently the active text editor.
 *      Most natural when the user is editing one doc and toggling
 *      its preview from the command palette.
 *   2. The single open panel, if there's exactly one.
 *   3. Otherwise undefined — surfaces an info message.
 */
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
    `Aozora Preview — ${document.fileName.split("/").pop() ?? "untitled"}`,
    vscode.ViewColumn.Beside,
    {
      // The preview only ever shows a static HTML+CSS fragment the LSP
      // renders from the document (no client-side script, no
      // `acquireVsCodeApi`, no `postMessage`). Keeping scripts OFF means
      // that even if the renderer ever emitted an unescaped tag, an
      // injected `<script>` could not execute — defense-in-depth over
      // aozora-render's HTML escaping, and consistent with the
      // `enableScripts: false` notation guide. Mirrors the strict CSP
      // the upstream aozora playground adopted (aozora#67).
      enableScripts: false,
      // Nothing is loaded from disk, so lock the resource roots to none.
      localResourceRoots: [],
      retainContextWhenHidden: true,
    },
  );

  const state: PreviewState = {
    panel,
    uri: document.uri,
    mode: configuredWritingMode(),
  };
  panelsByUri.set(key, state);

  panel.onDidDispose(
    () => {
      const live = panelsByUri.get(key);
      if (live?.debounce) {
        clearTimeout(live.debounce);
      }
      panelsByUri.delete(key);
    },
    undefined,
    context.subscriptions,
  );

  await renderInto(state, client);
}

async function renderInto(state: PreviewState, client: LanguageClient): Promise<void> {
  try {
    const result = await client.sendRequest<RenderHtmlResult>("aozora/renderHtml", {
      uri: state.uri.toString(),
    });
    state.panel.webview.html = wrapHtml(result.html ?? "", state.mode);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    state.panel.webview.html = wrapHtml(
      `<pre>aozora/renderHtml failed: ${escapeHtml(message)}</pre>`,
      state.mode,
    );
  }
}

/**
 * Wrap the LSP-rendered body fragment in a minimal HTML5 host page
 * with a stylesheet that gives 青空文庫 prose a comfortable default
 * appearance. The base ruleset targets 横書き (left-to-right);
 * `mode === "vertical"` adds an override that flips to
 * `writing-mode: vertical-rl`, the right-to-left top-to-bottom layout
 * Aozora Bunko texts use in print.
 *
 * Vertical mode quirks worth knowing:
 *
 * - `max-width: 42em` becomes a *height* limit when the writing mode
 *   flips, which would chop tall content. We unset both `max-width`
 *   and `max-height` and pin a fixed `height: calc(100vh - 3em)` so
 *   the WebView's natural horizontal scrollbar handles overflow.
 * - The canonical stylesheet's `.aozora-vertical` hook (applied to
 *   `<body>`) supplies `writing-mode` / `text-orientation`; 縦中横
 *   combines correctly via its `.aozora-combine-upright {
 *   text-combine-upright: all }` rule.
 */
function wrapHtml(body: string, mode: WritingMode): string {
  // writing-mode / text-orientation come from the canonical
  // `.aozora-vertical` hook on <body>; only the WebView container quirks
  // for vertical layout live here — a `max-width` would become a *height*
  // clamp once the writing mode flips and chop tall content, so unset it
  // and pin a viewport-height box the WebView scrolls horizontally.
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
  <!-- Strict CSP, defense-in-depth over the renderer's escaping: no
       scripts of any origin, no remote/data anything except inline
       styles (the <style> block below) and data: images. Combined with
       enableScripts:false this makes injected markup inert even if the
       LSP fragment ever carried an unescaped tag. -->
  <meta http-equiv="Content-Security-Policy"
    content="default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'" />
  <title>Aozora Preview</title>
  <style>
    /* Follow the editor theme instead of a hardcoded page colour; the
       prose font / colour / line-height come from .aozora-notation. */
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
