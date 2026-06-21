// `Aozora: Canonicalize slug at cursor` — palette entry that snaps the
// `［＃...］` slug under the cursor to its canonical form (e.g.
// `［＃ぼうてん］` → `［＃傍点］`) by delegating to the LSP's
// `aozora.canonicalizeSlug` workspace command.
//
// The LSP server already implements the full canonicalisation logic
// (see `crates/aozora-lsp/src/commands.rs`); the extension just needs
// to (1) locate the slug span containing the cursor and (2) forward
// `{uri, range, body}` over `workspace/executeCommand`. The server
// returns a `WorkspaceEdit` and applies it via `workspace/applyEdit`.

import { commands, type ExtensionContext, Range, window } from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

// Match a single slug body. Matches both half-width `[#…]` and
// full-width `［＃…］` openings — the LSP `strip_brackets` helper is
// permissive in the same way.
const SLUG_PATTERN = /[［[][＃#][^］\]\n]*[］\]]/g;

export function registerCanonicalizeAtCursorCommand(
  context: ExtensionContext,
  client: LanguageClient,
): void {
  context.subscriptions.push(
    commands.registerCommand("aozora.canonicalizeSlugAtCursor", async () => {
      const editor = window.activeTextEditor;
      if (!editor || editor.document.languageId !== "aozora") {
        void window.showInformationMessage(
          "Aozora ファイル上にカーソルを置いてから実行してください。",
        );
        return;
      }

      const position = editor.selection.active;
      const line = editor.document.lineAt(position.line);
      const lineText = line.text;
      const cursorCol = position.character;

      let target: { range: Range; body: string } | undefined;
      for (const m of lineText.matchAll(SLUG_PATTERN)) {
        const start = m.index ?? 0;
        const end = start + m[0].length;
        // Inclusive on both ends so the cursor sitting on the closing
        // ］ still counts as "inside the slug".
        if (cursorCol >= start && cursorCol <= end) {
          target = {
            range: new Range(position.line, start, position.line, end),
            body: m[0],
          };
          break;
        }
      }

      if (!target) {
        void window.showInformationMessage("カーソル位置に ［＃...］ 注記が見つかりませんでした。");
        return;
      }

      try {
        await client.sendRequest("workspace/executeCommand", {
          command: "aozora.canonicalizeSlug",
          arguments: [
            {
              uri: editor.document.uri.toString(),
              range: {
                start: { line: target.range.start.line, character: target.range.start.character },
                end: { line: target.range.end.line, character: target.range.end.character },
              },
              body: target.body,
            },
          ],
        });
        // Server returns null when the slug is already canonical or
        // unrecognised. We don't need to act on the response — the
        // edit (if any) is applied via `workspace/applyEdit`.
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        void window.showErrorMessage(`canonicalize に失敗: ${message}`);
      }
    }),
  );
}
