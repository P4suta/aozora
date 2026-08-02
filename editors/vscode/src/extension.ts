// aozora VS Code extension — launches the aozora language server over
// stdio (`aozora lsp --stdio`, in-process inside the `aozora` binary) and
// wires it to .afm / .aozora / .aozora.txt documents, plus any plaintext
// .txt file whose content looks like an aozora-bunko work.

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

let client: LanguageClient | undefined;

export async function activate(context: ExtensionContext): Promise<void> {
  const config = workspace.getConfiguration("aozora");
  const defaultBinary = resolveDefaultBinary(context);
  const lspPath = resolveLspBinary(config, defaultBinary);

  // The language server is the `aozora` binary's `lsp` subcommand, run
  // in-process. `--stdio` is accepted for editor compatibility; stdio is
  // the only transport.
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
          // biome-ignore lint/style/useNamingConvention: env var name is an external contract
          AOZORA_LOG: "aozora_cli=debug",
        },
      },
    },
  };

  const clientOptions: LanguageClientOptions = {
    // Match aozora language docs across BOTH on-disk files (`file://`)
    // AND scratch buffers the user hasn't saved yet (`untitled:`).
    // Without `untitled`, the LSP client never syncs Untitled-N
    // buffers — `textDocument/didOpen` is gated on the selector — so
    // every server-side feature (onType conversion, hover, gaiji
    // spans, renderHtml preview, document symbols, formatting,
    // semantic tokens) silently no-ops with `-32602 no document at
    // uri` until the user saves. This is the standard
    // file + untitled pattern shipped by rust-analyzer,
    // typescript-language-features, and other major LSP clients.
    documentSelector: [
      { scheme: "file", language: "aozora" },
      { scheme: "untitled", language: "aozora" },
    ],
  };

  client = new LanguageClient("aozora", "aozora Language Server", serverOptions, clientOptions);

  context.subscriptions.push(client);

  // Manual opt-in: users can force any open document into the aozora
  // language, handy for .txt scratch buffers that don't trigger the
  // auto-detect heuristic below.
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

  // Auto-detect: scan plaintext .txt documents at open time and flip
  // them to "aozora" if the first few kilobytes look like an aozora
  // bunko work. The aozora-bunko input manual doesn't mandate a file
  // extension (工作員 typically save as .txt), so relying on extension
  // alone misses most real-world files.
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

  // Wire the preview pane command BEFORE starting the client — the
  // command itself only sends a request after `client.start()` resolves
  // but registering the disposable up front matches the pattern every
  // other contribution in this file uses.
  registerPreviewCommand(context, client);
  // Wrap-selection commands are pure client-side WorkspaceEdits: no
  // LSP roundtrip needed for trivial open/close splices. The same
  // wraps are also exposed via the LSP `code_action` handler so
  // editors that talk LSP only (helix, neovim) get them too.
  registerWrapCommands(context);
  // Inline-fold gaiji spans — `※[#…]` collapses to its resolved
  // glyph when the cursor is elsewhere; expands back when cursor
  // enters the span. Driven by the LSP `aozora/gaijiSpans` request.
  registerGaijiFold(context, client);
  // `Aozora: 記法ガイドを開く` — opens aozora-bunko's own annotation
  // reference in the browser. Upstream owns the notation, so we link
  // to it rather than restate it. Discoverable from the command
  // palette and the editor context menu.
  context.subscriptions.push(
    commands.registerCommand("aozora.openNotationGuide", async () => {
      await env.openExternal(Uri.parse("https://www.aozora.gr.jp/annotation/"));
    }),
  );

  // Discoverability shortcuts for the auto-wired LSP features
  // (foldingRange / documentSymbol / semanticTokens). VS Code picks
  // those up from the LSP capabilities automatically — the symbols
  // panel populates, the gutter shows fold chevrons, the editor
  // colours ruby/gaiji per the active theme — but new users don't
  // necessarily know the built-in keybindings (Ctrl+Shift+O for the
  // outline picker, etc). The proxy commands below appear in the
  // Command Palette under the "Aozora:" prefix so they're findable
  // alongside our other contributions.
  registerLspFeatureShortcuts(context);

  // IDE-style aggressive auto-expansion: type ｜ → snippet
  // `｜<base>《<reading>》` with `<base>` selected and Tab navigation;
  // type ［ → `［<cursor>］` with close auto-paired; type ］ next to
  // an existing close → skip-over instead of double-insert. The
  // snippet placeholder semantics are VS-Code-only (LSP onType
  // returns plain TextEdits), so this lives here on the extension
  // side and complements (rather than replaces) the LSP onType for
  // non-VS-Code clients.
  registerSnippetTriggers(context);

  // Auto-delete an empty bracket pair when the user empties its
  // contents (e.g. types `#` to wrap as `［＃］`, then Backspaces the
  // `＃` and expects `［］` to also vanish). Complements VS Code's
  // built-in `editor.autoClosingDelete: "always"`, which only
  // covers the inverse direction (delete-open → close also goes).
  registerDeletePair(context);

  // Custom outline picker. Renders the LSP's documentSymbol
  // response (大/中/小 見出し) into a `window.showQuickPick` UI;
  // robust against the editor-focus quirks that broke the prior
  // built-in proxy.
  registerShowOutlineCommand(context);

  // `Aozora: Canonicalize slug at cursor` — bridges the LSP's
  // `aozora.canonicalizeSlug` workspace command into the palette so
  // users can normalise `［＃ぼうてん］` → `［＃傍点］` without
  // hunting for the lightbulb.
  registerCanonicalizeAtCursorCommand(context, client);

  // `Aozora: Export HTML…` renders the document to a standalone HTML file via
  // the LSP, and `Aozora: Lint workspace` runs the `aozora` CLI's batch linter
  // — bringing the new CLI surfaces (render / lint) into the editor.
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

// Binary resolution order:
//
//   1. If the user explicitly set `aozora.lsp.path` to a non-default
//      value, honour that — they want their own build.
//   2. Otherwise prefer the bundled `server/aozora(.exe)` shipped
//      inside this extension's platform-specific .vsix. This is the
//      zero-config path that hits the moment a fresh user installs.
//   3. Fall back to looking up `aozora` on the user's PATH — covers
//      the case of a platform-neutral .vsix install (no bundled
//      binary) where the user manually `cargo install`-ed the CLI.
//
// The resolved binary is always invoked as `<binary> lsp --stdio`: the
// language server is a subcommand of the `aozora` CLI, not a program of
// its own.
//
// On Unix we also chmod 0755 the bundled binary on first activation:
// vsce ships .vsix archives via a zip codepath that doesn't preserve
// the executable bit (rust-analyzer hits the same issue). The chmod
// is idempotent and cheap.
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
      } catch {
        // Non-fatal: if chmod fails the launch will surface its own
        // error and the user can fix permissions manually.
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

/// Wire three thin proxies that delegate to VS Code built-in
/// commands. Their value is **discoverability**: the shortcuts
/// appear in the Command Palette under our `Aozora:` prefix so a
/// user typing "aozora" sees outline / fold-all / unfold-all next
/// to the existing wrap-with-ruby etc commands. Without these,
/// the LSP-driven features (documentSymbol, foldingRange) work
/// but new users don't know to press the built-in shortcut keys.
function registerLspFeatureShortcuts(context: ExtensionContext): void {
  const proxy = (cmdId: string, builtin: string): void => {
    context.subscriptions.push(
      commands.registerCommand(cmdId, async () => {
        await commands.executeCommand(builtin);
      }),
    );
  };
  // `aozora.showOutline` is implemented in `outline.ts` against
  // `vscode.executeDocumentSymbolProvider` — proxying
  // `workbench.action.gotoSymbol` (or `editor.action.quickOutline`)
  // here was unreliable: when the command is dispatched from a
  // non-editor focus (palette / title bar / side bar) the built-in
  // opens Quick Open with the `@` prefix but no editor context,
  // and the symbol list silently stays empty.
  proxy("aozora.foldAll", "editor.foldAll");
  proxy("aozora.unfoldAll", "editor.unfoldAll");
}

// Minimal VS Code variable resolver for the few substitutions that
// matter inside `aozora.lsp.path` — enough to let settings.json carry
// `${workspaceFolder}/../target/release/aozora` instead of a hard-
// coded absolute path.
function resolveVars(input: string): string {
  const ws = workspace.workspaceFolders?.[0]?.uri.fsPath ?? "";
  return input
    .replace(/\$\{workspaceFolder\}/g, ws)
    .replace(/\$\{userHome\}/g, homedir())
    .replace(/\$\{env:([A-Za-z_][A-Za-z0-9_]*)\}/g, (_, name) => process.env[name] ?? "");
}

// Aozora-bunko "input manual" feature detection.
//
// We look at the leading ~4 KiB of the buffer for four independent
// signals:
//
//   1. `［＃...］` — any editor annotation.
//   2. `｜X《Y》` — explicit ruby form, cheap to match reliably.
//   3. `X《Y》` — implicit ruby form, matched with a kanji-range base.
//   4. the standard `-------...` header-separator line, ≥ 40 dashes,
//      which the aozora-bunko preamble uses to fence the legend block.
//
// Any one of those is enough; the heuristic is tuned for high precision
// on typical aozora source files rather than maximum recall.
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
  const head = document.getText(new Range(new Position(0, 0), document.positionAt(4096)));
  if (!looksLikeAozora(head)) {
    return;
  }
  await languages.setTextDocumentLanguage(document, "aozora");
}
