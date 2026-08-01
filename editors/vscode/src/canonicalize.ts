import { commands, type ExtensionContext, Range, window } from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

import { canonicalizeSnapshotMatches, findSlugAtOffset } from "./extensionLogic";

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
      const version = editor.document.version;
      const line = editor.document.lineAt(position.line);
      const lineText = line.text;
      const cursorCol = position.character;

      const match = findSlugAtOffset(lineText, cursorCol);
      const target = match
        ? {
            range: new Range(position.line, match.start, position.line, match.end),
            body: match.body,
          }
        : undefined;

      if (!target) {
        void window.showInformationMessage("カーソル位置に ［＃...］ 注記が見つかりませんでした。");
        return;
      }

      if (
        !canonicalizeSnapshotMatches(
          version,
          editor.document.version,
          target.body,
          editor.document.getText(target.range),
        )
      ) {
        void window.showInformationMessage("文書が変更されました。もう一度実行してください。");
        return;
      }

      try {
        await client.sendRequest("workspace/executeCommand", {
          command: "aozora.canonicalizeSlug",
          arguments: [
            {
              uri: editor.document.uri.toString(),
              version,
              range: {
                start: { line: target.range.start.line, character: target.range.start.character },
                end: { line: target.range.end.line, character: target.range.end.character },
              },
              body: target.body,
            },
          ],
        });
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        void window.showErrorMessage(`canonicalize に失敗: ${message}`);
      }
    }),
  );
}
