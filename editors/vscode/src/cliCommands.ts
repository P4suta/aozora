// aozora CLI-flavoured commands for the editor.
//
// `aozora.exportHtml` renders the active document to a standalone HTML file the
// user picks — reusing the LSP `aozora/renderHtml` request (the same one the
// preview pane uses), so it works with zero extra binaries.
//
// `aozora.lintWorkspace` runs the `aozora` CLI's terminal linter over the
// workspace folder for batch diagnostics beyond the open editors (the live LSP
// only diagnoses documents the editor has opened).

import { homedir } from "node:os";
import { dirname, join } from "node:path";

import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";
import { documentVersionMatches, htmlFileName } from "./extensionLogic";
import { parseRenderHtmlResult } from "./lspWire";
import { aozoraNotationStyles } from "./notationStyles";
import { WorkspaceLintTerminal } from "./workspaceLint";
import { WORKSPACE_LINT_FILE_LIMIT } from "./workspaceLintLogic";

export function registerCliCommands(
  context: vscode.ExtensionContext,
  client: LanguageClient,
  bundledCli: string,
): void {
  context.subscriptions.push(
    vscode.commands.registerCommand("aozora.exportHtml", () => exportHtml(client)),
  );
  context.subscriptions.push(
    vscode.commands.registerCommand("aozora.lintWorkspace", () => lintWorkspace(bundledCli)),
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
  const version = document.version;

  let result: ReturnType<typeof parseRenderHtmlResult>;
  try {
    result = parseRenderHtmlResult(
      await client.sendRequest<unknown>("aozora/renderHtml", {
        uri: document.uri.toString(),
      }),
    );
  } catch (err) {
    void vscode.window.showErrorMessage(`aozora: render failed: ${asMessage(err)}`);
    return;
  }
  if (!documentVersionMatches(version, document.version)) {
    void vscode.window.showWarningMessage(
      "aozora: the document changed while rendering; run Export HTML again.",
    );
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
  const html = wrapStandalone(documentTitle(document.uri), result.html);

  const defaultUri = defaultHtmlUri(document.uri);
  const target = await vscode.window.showSaveDialog({
    saveLabel: "Export HTML",
    ...(defaultUri ? { defaultUri } : {}),
    // biome-ignore lint/style/useNamingConvention: VS Code shows the filter key as its display label
    filters: { HTML: ["html"] },
  });
  if (!target) {
    return;
  }
  if (!documentVersionMatches(version, document.version)) {
    void vscode.window.showWarningMessage(
      "aozora: the document changed before export; run Export HTML again.",
    );
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

async function lintWorkspace(bundledCli: string): Promise<void> {
  const activeDocument = vscode.window.activeTextEditor?.document;
  const activeFolder = activeDocument
    ? vscode.workspace.getWorkspaceFolder(activeDocument.uri)
    : undefined;
  const firstFolder = vscode.workspace.workspaceFolders?.[0];
  const folder = activeFolder ?? firstFolder;
  const activeFile = activeDocument?.uri.scheme === "file" ? activeDocument.uri.fsPath : undefined;
  const files = folder
    ? await workspaceLintFiles(folder)
    : activeFile
      ? [vscode.Uri.file(activeFile)]
      : [];
  if (files.length === 0) {
    void vscode.window.showInformationMessage("Open a folder or a file to lint.");
    return;
  }
  const firstFile = files[0];
  if (!firstFile) {
    return;
  }
  if (files.length > WORKSPACE_LINT_FILE_LIMIT) {
    void vscode.window.showWarningMessage(
      `Aozora workspace lint supports at most ${WORKSPACE_LINT_FILE_LIMIT} files per run.`,
    );
    return;
  }
  const configured = vscode.workspace
    .getConfiguration("aozora", folder?.uri ?? activeDocument?.uri)
    .get<string>("cli.path", "")
    .trim();
  const bin = configured.length > 0 ? resolveCliPath(configured, folder) : bundledCli;
  const cwd = folder?.uri.fsPath ?? dirname(firstFile.fsPath);
  const execution = new vscode.CustomExecution(
    async () => new WorkspaceLintTerminal(bin, files, cwd),
  );
  const definition = { type: "aozora", files: files.length };
  const task = new vscode.Task(
    definition,
    folder ?? vscode.TaskScope.Workspace,
    folder ? "lint workspace" : "lint file",
    "aozora",
    execution,
    ["$aozora-lint"],
  );
  task.presentationOptions = {
    clear: true,
    panel: vscode.TaskPanelKind.Dedicated,
    reveal: vscode.TaskRevealKind.Always,
  };
  try {
    await vscode.tasks.executeTask(task);
  } catch (err) {
    void vscode.window.showErrorMessage(
      `aozora: could not start workspace lint: ${asMessage(err)}`,
    );
  }
}

async function workspaceLintFiles(folder: vscode.WorkspaceFolder): Promise<vscode.Uri[]> {
  const pattern = new vscode.RelativePattern(folder, "**/*.{afm,aozora,txt,text}");
  const exclude = new vscode.RelativePattern(
    folder,
    "**/{.git,node_modules,target,out,dist,.next,coverage}/**",
  );
  const files = await vscode.workspace.findFiles(pattern, exclude, WORKSPACE_LINT_FILE_LIMIT + 1);
  return files
    .filter((uri) => uri.scheme === "file")
    .sort((left, right) => (left.fsPath < right.fsPath ? -1 : left.fsPath > right.fsPath ? 1 : 0));
}

function resolveCliPath(value: string, folder: vscode.WorkspaceFolder | undefined): string {
  return value
    .replace(/\$\{workspaceFolder\}/g, folder?.uri.fsPath ?? "")
    .replace(/\$\{userHome\}/g, homedir())
    .replace(/\$\{env:([A-Za-z_][A-Za-z0-9_]*)\}/g, (_, name: string) => process.env[name] ?? "");
}

function documentTitle(uri: vscode.Uri): string {
  return uri.path.split("/").pop() ?? "aozora";
}

function defaultHtmlUri(source: vscode.Uri): vscode.Uri | undefined {
  if (source.scheme === "file") {
    return vscode.Uri.file(join(dirname(source.fsPath), htmlFileName(source.fsPath)));
  }
  const folder = vscode.workspace.workspaceFolders?.[0];
  return folder ? vscode.Uri.joinPath(folder.uri, htmlFileName(source.path)) : undefined;
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
