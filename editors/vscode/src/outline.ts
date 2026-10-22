import {
  commands,
  type DocumentSymbol,
  type ExtensionContext,
  type QuickPickItem,
  type Range,
  Selection,
  TextEditorRevealType,
  window,
} from "vscode";
import { documentVersionMatches } from "./extensionLogic";

const LANG_ID = "aozora";

interface SymbolPick extends QuickPickItem {
  range: Range;
}

export function registerShowOutlineCommand(context: ExtensionContext): void {
  context.subscriptions.push(
    commands.registerCommand("aozora.showOutline", async () => {
      const editor = window.activeTextEditor;
      if (!editor) {
        void window.showInformationMessage(
          "アウトラインを表示するには、まず .afm ファイルを開いてください。",
        );
        return;
      }
      if (editor.document.languageId !== LANG_ID) {
        void window.showInformationMessage(
          "アクティブなファイルが Aozora 言語ではありません。右下から言語モードを切り替えてください。",
        );
        return;
      }
      const document = editor.document;
      const version = document.version;

      const symbols = await commands.executeCommand<DocumentSymbol[]>(
        "vscode.executeDocumentSymbolProvider",
        document.uri,
      );

      if (!documentVersionMatches(version, document.version)) {
        void window.showInformationMessage(
          "文書が変更されたため、アウトラインをもう一度開いてください。",
        );
        return;
      }

      if (!symbols || symbols.length === 0) {
        void window.showInformationMessage(
          "見出しが見つかりませんでした。\n" +
            "［＃大見出し］ / ［＃中見出し］ / ［＃小見出し］ を本文中に置くと、ここに一覧されます。",
        );
        return;
      }

      const items: SymbolPick[] = [];
      const walk = (group: readonly DocumentSymbol[], depth: number): void => {
        for (const s of group) {
          const item: SymbolPick = {
            label: `${"  ".repeat(depth)}${s.name}`,
            range: s.range,
          };
          if (s.detail.length > 0) {
            item.description = s.detail;
          }
          items.push(item);
          if (s.children.length > 0) {
            walk(s.children, depth + 1);
          }
        }
      };
      walk(symbols, 0);

      const picked = await window.showQuickPick(items, {
        placeHolder: "見出しを選択してジャンプ",
        matchOnDescription: true,
      });
      if (!picked) {
        return;
      }
      if (!documentVersionMatches(version, document.version)) {
        void window.showInformationMessage(
          "文書が変更されたため、アウトラインをもう一度開いてください。",
        );
        return;
      }

      const targetEditor = await window.showTextDocument(document, editor.viewColumn);
      if (!documentVersionMatches(version, document.version)) {
        void window.showInformationMessage(
          "文書が変更されたため、アウトラインをもう一度開いてください。",
        );
        return;
      }
      const head = picked.range.start;
      targetEditor.selection = new Selection(head, head);
      targetEditor.revealRange(picked.range, TextEditorRevealType.InCenter);
    }),
  );
}
