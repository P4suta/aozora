// aozora CLI-flavoured commands for the editor.
//
// `aozora.exportHtml` renders the active document to a standalone HTML file the
// user picks — reusing the LSP `aozora/renderHtml` request (the same one the
// preview pane uses), so it works with zero extra binaries.
//
// `aozora.lintWorkspace` runs the `aozora` CLI's terminal linter over the
// workspace folder for batch diagnostics beyond the open editors (the live LSP
// only diagnoses documents the editor has opened). It needs the `aozora` CLI on
// PATH, or `aozora.cli.path` set.

import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

import type { RenderHtmlResult } from "./lspWire";
import { aozoraNotationStyles } from "./notationStyles";

export function registerCliCommands(
  context: vscode.ExtensionContext,
  client: LanguageClient,
): void {
  context.subscriptions.push(
    vscode.commands.registerCommand("aozora.exportHtml", () => exportHtml(client)),
  );
  context.subscriptions.push(
    vscode.commands.registerCommand("aozora.lintWorkspace", () => lintWorkspace()),
  );
}

async function exportHtml(client: LanguageClient): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.languageId !== "aozora") {
    void vscode.window.showInformationMessage(
      "Open an aozora document first, then run this command.",
    );
    return;
  }
  const document = editor.document;

  let result: RenderHtmlResult;
  try {
    result = await client.sendRequest<RenderHtmlResult>("aozora/renderHtml", {
      uri: document.uri.toString(),
    });
  } catch (err) {
    void vscode.window.showErrorMessage(`aozora: render failed: ${asMessage(err)}`);
    return;
  }

  // A paused render carries a notice instead of the document. Stop
  // before the save dialog: exporting it would write a placeholder to
  // the user's chosen path and report success.
  if (result.paused) {
    void vscode.window.showWarningMessage(
      "aozora: this document is too large for the server to render, so there is nothing to export.",
    );
    return;
  }
  const html = wrapStandalone(documentTitle(document.uri), result.html ?? "");

  const target = await vscode.window.showSaveDialog({
    saveLabel: "Export HTML",
    defaultUri: defaultHtmlUri(document.uri),
    // biome-ignore lint/style/useNamingConvention: VS Code shows the filter key as its display label
    filters: { HTML: ["html"] },
  });
  if (!target) {
    return;
  }

  try {
    await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(html));
  } catch (err) {
    void vscode.window.showErrorMessage(
      `aozora: could not write ${target.fsPath}: ${asMessage(err)}`,
    );
    return;
  }

  const open = "Open";
  const choice = await vscode.window.showInformationMessage(
    `Exported HTML to ${target.fsPath}`,
    open,
  );
  if (choice === open) {
    void vscode.env.openExternal(target);
  }
}

function lintWorkspace(): void {
  const folder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  const target = folder ?? vscode.window.activeTextEditor?.document.uri.fsPath;
  if (target === undefined) {
    void vscode.window.showInformationMessage("Open a folder or a file to lint.");
    return;
  }
  const bin =
    vscode.workspace.getConfiguration("aozora").get<string>("cli.path", "").trim() || "aozora";

  const terminal = vscode.window.createTerminal("aozora lint");
  terminal.show();
  // The terminal surfaces the rustc-style output (and any "command not
  // found" if the CLI is not installed); paths are click-to-open there.
  terminal.sendText(`${bin} lint "${target}"`);
}

function documentTitle(uri: vscode.Uri): string {
  return uri.path.split("/").pop() ?? "aozora";
}

function defaultHtmlUri(source: vscode.Uri): vscode.Uri {
  const base = source.fsPath.replace(/\.(afm|aozora|aozora\.txt|txt)$/i, "");
  return vscode.Uri.file(`${base}.html`);
}

function asMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/**
 * Wrap an LSP-rendered body fragment in a self-contained HTML5 document with
 * vertical-writing (縦書き) CSS — the standalone form for sharing or printing,
 * matching `aozora render --standalone`.
 */
function wrapStandalone(title: string, body: string): string {
  return `<!DOCTYPE html>
<html lang="ja">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>${escapeHtml(title)}</title>
<style>
  /* Self-contained share / print form. Prose font / colour come from
     .aozora-notation; 縦書き from .aozora-vertical (both on <body>). */
  body {
    max-block-size: 40em;
    margin: 1.5em auto;
    padding: 0 1em;
    background: #fdf6e3;
  }
  ${aozoraNotationStyles}
</style>
</head>
<body class="aozora-notation aozora-vertical">
${body}
</body>
</html>
`;
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
