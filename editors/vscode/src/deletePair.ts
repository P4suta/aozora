import {
  type ExtensionContext,
  Position,
  Range,
  type TextDocumentChangeEvent,
  window,
  workspace,
} from "vscode";

const LANG_ID = "aozora";

const BRACKET_PAIRS: ReadonlyMap<string, string> = new Map([
  ["［", "］"],
  ["《", "》"],
  ["「", "」"],
  ["『", "』"],
  ["〔", "〕"],
  ["（", "）"],
  ["【", "】"],
  ["｛", "｝"],
]);

export function registerDeletePair(context: ExtensionContext): void {
  let busy = false;

  const handler = async (event: TextDocumentChangeEvent): Promise<void> => {
    if (busy) {
      return;
    }
    if (event.document.languageId !== LANG_ID) {
      return;
    }
    const editor = window.activeTextEditor;
    if (!editor || editor.document !== event.document) {
      return;
    }

    if (event.contentChanges.length !== 1) {
      return;
    }
    const change = event.contentChanges[0];
    if (!change) {
      return;
    }
    if (change.text !== "" || change.rangeLength === 0) {
      return;
    }

    const line = change.range.start.line;
    const cursorCol = change.range.start.character;
    if (cursorCol === 0) {
      return;
    }
    const lineText = event.document.lineAt(line).text;
    const openCandidate = lineText.charAt(cursorCol - 1);
    const closeCandidate = lineText.charAt(cursorCol);
    if (BRACKET_PAIRS.get(openCandidate) !== closeCandidate) {
      return;
    }

    busy = true;
    try {
      const pairRange = new Range(
        new Position(line, cursorCol - 1),
        new Position(line, cursorCol + 1),
      );
      await editor.edit((eb) => eb.delete(pairRange));
    } finally {
      busy = false;
    }
  };

  context.subscriptions.push(workspace.onDidChangeTextDocument((e) => void handler(e)));
}
