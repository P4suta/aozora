import { chmodSync, existsSync, constants as fsConstants, statSync } from "node:fs";
import { homedir } from "node:os";
import { join as pathJoin } from "node:path";
import {
  commands,
  type ExtensionContext,
  env,
  languages,
  Position,
  Range,
  type TextDocument,
  Uri,
  type WorkspaceConfiguration,
  window,
  workspace,
} from "vscode";
import {
  LanguageClient,
  type LanguageClientOptions,
  type ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

import { registerCanonicalizeAtCursorCommand } from "./canonicalize";
import { registerCliCommands } from "./cliCommands";
import { registerDeletePair } from "./deletePair";
import { registerGaijiFold } from "./gaijiFold";
import { registerShowOutlineCommand } from "./outline";
import { registerPreviewCommand } from "./preview";
import { registerSnippetTriggers } from "./snippetTrigger";
import { registerWrapCommands } from "./wrap";

const NOTATION_GUIDE_URL = "https://www.aozora.gr.jp/annotation/";
const AUTO_DETECT_SNIFF_LENGTH = 4096;

let client: LanguageClient | undefined;

export async function activate(context: ExtensionContext): Promise<void> {
  const config = workspace.getConfiguration("aozora");
  const defaultBinary = resolveDefaultBinary(context);
  const lspPath = resolveLspBinary(config, defaultBinary);

  const serverArgs = ["lsp", "--stdio"];
  const serverOptions: ServerOptions = {
    run: {
      command: lspPath,
      args: serverArgs,
      transport: TransportKind.stdio,
    },
    debug: {
      command: lspPath,
      args: serverArgs,
      transport: TransportKind.stdio,
      options: {
        env: {
          ...process.env,
          // biome-ignore lint/style/useNamingConvention: The environment variable name is an external contract.
          AOZORA_LOG: "aozora_cli=debug",
        },
      },
    },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "aozora" },
      { scheme: "untitled", language: "aozora" },
    ],
  };

  client = new LanguageClient("aozora", "aozora Language Server", serverOptions, clientOptions);

  context.subscriptions.push(client);

  context.subscriptions.push(
    commands.registerCommand("aozora.setLanguageMode", async () => {
      const editor = window.activeTextEditor;
      if (!editor) {
        void window.showInformationMessage("Open a text editor first, then run this command.");
        return;
      }
      await languages.setTextDocumentLanguage(editor.document, "aozora");
    }),
  );

  const autoDetect = (document: TextDocument) => {
    void maybeSwitchToAozora(document).catch((err: unknown) => {
      const message = err instanceof Error ? err.message : String(err);
      void window.showErrorMessage(`aozora language detection failed: ${message}`);
    });
  };
  context.subscriptions.push(workspace.onDidOpenTextDocument(autoDetect));
  for (const doc of workspace.textDocuments) {
    autoDetect(doc);
  }

  registerPreviewCommand(context, client);
  registerWrapCommands(context);
  registerGaijiFold(context, client);
  context.subscriptions.push(
    commands.registerCommand("aozora.openNotationGuide", async () => {
      await env.openExternal(Uri.parse(NOTATION_GUIDE_URL));
    }),
  );

  registerLspFeatureShortcuts(context);

  registerSnippetTriggers(context);

  registerDeletePair(context);

  registerShowOutlineCommand(context);

  registerCanonicalizeAtCursorCommand(context, client);

  registerCliCommands(context, client, defaultBinary);

  try {
    await client.start();
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    void window.showErrorMessage(
      `aozora language server failed to start (${lspPath} lsp --stdio): ${message}. ` +
        "Set `aozora.lsp.path` in Settings if you have a custom build, " +
        "or reinstall the extension to restore the bundled server binary.",
    );
  }
}

function resolveLspBinary(config: WorkspaceConfiguration, defaultBinary: string): string {
  const userSetting = config.get<string>("lsp.path", "aozora").trim();
  if (userSetting !== "" && userSetting !== "aozora") {
    return resolveVars(userSetting);
  }

  return defaultBinary;
}

function resolveDefaultBinary(context: ExtensionContext): string {
  const exe = process.platform === "win32" ? "aozora.exe" : "aozora";
  const bundled = pathJoin(context.extensionPath, "server", exe);
  if (existsSync(bundled)) {
    if (process.platform !== "win32") {
      try {
        const mode = statSync(bundled).mode;
        const wantBits = fsConstants.S_IXUSR | fsConstants.S_IXGRP | fsConstants.S_IXOTH;
        if ((mode & wantBits) !== wantBits) {
          chmodSync(bundled, mode | wantBits);
        }
      } catch (err) {
        console.error(
          `Failed to set executable permissions for bundled aozora server ${bundled}`,
          err,
        );
      }
    }
    return bundled;
  }

  return "aozora";
}

export async function deactivate(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}

function registerLspFeatureShortcuts(context: ExtensionContext): void {
  const proxy = (cmdId: string, builtin: string): void => {
    context.subscriptions.push(
      commands.registerCommand(cmdId, async () => {
        await commands.executeCommand(builtin);
      }),
    );
  };
  proxy("aozora.foldAll", "editor.foldAll");
  proxy("aozora.unfoldAll", "editor.unfoldAll");
}

function resolveVars(input: string): string {
  const ws = workspace.workspaceFolders?.[0]?.uri.fsPath ?? "";
  return input
    .replace(/\$\{workspaceFolder\}/g, ws)
    .replace(/\$\{userHome\}/g, homedir())
    .replace(/\$\{env:([A-Za-z_][A-Za-z0-9_]*)\}/g, (_, name) => process.env[name] ?? "");
}

function looksLikeAozora(head: string): boolean {
  if (head.includes("［＃")) {
    return true;
  }
  if (/｜[^《》\n]{1,40}《[^》\n]{1,40}》/.test(head)) {
    return true;
  }
  if (/[\p{Script=Han}々〆ヵヶ]{1,20}《[^》\n]{1,40}》/u.test(head)) {
    return true;
  }
  if (/^-{40,}$/m.test(head)) {
    return true;
  }
  return false;
}

async function maybeSwitchToAozora(document: TextDocument): Promise<void> {
  if (document.languageId !== "plaintext") {
    return;
  }
  if (document.uri.scheme !== "file") {
    return;
  }
  const path = document.uri.fsPath;
  if (!/\.(txt|text)$/i.test(path)) {
    return;
  }
  const enabled = workspace.getConfiguration("aozora").get<boolean>("autoDetect.plaintext", true);
  if (!enabled) {
    return;
  }
  const head = document.getText(
    new Range(new Position(0, 0), document.positionAt(AUTO_DETECT_SNIFF_LENGTH)),
  );
  if (!looksLikeAozora(head)) {
    return;
  }
  await languages.setTextDocumentLanguage(document, "aozora");
}
