// `outline.ts` — `Aozora: アウトラインを表示` command.
//
// Why we don't proxy a built-in command:
//
// VS Code ships two flavours of "go to symbol": the in-editor
// `editor.action.quickOutline` and the workbench-level
// `workbench.action.gotoSymbol`. Both open the Quick Open prompt with
// the `@` symbol-filter prefix and let the editor's
// `DocumentSymbolProvider` populate the list.
//
// In practice, dispatching either of those via
// `commands.executeCommand` from a `Aozora: …` palette entry is
// fragile:
//
//   - When the title bar / side bar / explorer has focus instead of
//     the editor (the natural state when the user reaches for the
//     command palette or the editor-title icon), the command opens
//     the Quick Open input but VS Code's symbol provider sees no
//     active editor and the picker stays empty.
//   - The user then sees only `@` typed into the prompt with no
//     symbols listed and assumes the feature is broken.
//
// The recommended pattern (officially documented at
// https://code.visualstudio.com/api/references/commands —
// `vscode.executeDocumentSymbolProvider`) is to call into the
// provider directly with an explicit document URI, then render the
// result with `window.showQuickPick`. That gives:
//
//   - Deterministic provider context (we pass the URI explicitly)
//   - Explicit empty-state message ("見出しが見つかりませんでした")
//   - Indentation that mirrors heading nesting (大 → 中 → 小)
//   - Range-aware reveal that centers the heading in the viewport
//
// rust-analyzer / dart-code / vetur all use this pattern for their
// custom outline pickers.

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

      // `vscode.executeDocumentSymbolProvider` is the officially-
      // sanctioned passthrough that asks every registered
      // `DocumentSymbolProvider` for the URI's symbols. Our LSP
      // emits the hierarchical (`DocumentSymbol[]`) form via
      // `DocumentSymbolResponse::Nested`; the workspace API
      // preserves that shape so we get nested children to walk.
      const symbols = await commands.executeCommand<DocumentSymbol[]>(
        "vscode.executeDocumentSymbolProvider",
        editor.document.uri,
      );

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

      // Move the cursor to the heading line and recenter the
      // viewport. `showTextDocument` reaffirms editor focus so the
      // typed cursor sits at the new selection rather than staying
      // in the (now-dismissed) Quick Pick input.
      const head = picked.range.start;
      editor.selection = new Selection(head, head);
      editor.revealRange(picked.range, TextEditorRevealType.InCenter);
      void window.showTextDocument(editor.document, editor.viewColumn);
    }),
  );
}
