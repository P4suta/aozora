import {
  commands,
  type ExtensionContext,
  Position,
  Range,
  Selection,
  SnippetString,
  type TextDocument,
  type TextDocumentChangeEvent,
  window,
  workspace,
} from "vscode";
import { documentVersionMatches } from "./extensionLogic";

const LANG_ID = "aozora";

interface WrapRule {
  trigger: string;
  body: string;
  suppressIfNextIs?: string;
  postExpandSuggest?: boolean;
}

// biome-ignore-start lint/suspicious/noTemplateCurlyInString: the editor interprets these placeholders as snippet syntax.
const WRAPS: readonly WrapRule[] = [
  {
    trigger: "#",
    body: "［＃${0}］",
    postExpandSuggest: true,
  },
  {
    trigger: "［",
    body: "［${0}］",
    suppressIfNextIs: "］",
  },
  {
    trigger: "《",
    body: "《${1:reading}》${0}",
    suppressIfNextIs: "》",
  },
  {
    trigger: "｜",
    body: "｜${1:base}《${2:reading}》${0}",
  },
  {
    trigger: "※",
    body: "※［＃「${1:description}」、${2:mencode}］${0}",
  },
  {
    trigger: "〔",
    body: "〔${0}〕",
    suppressIfNextIs: "〕",
  },
];
// biome-ignore-end lint/suspicious/noTemplateCurlyInString: the editor interprets these placeholders as snippet syntax.

const SKIP_OVER_CHARS: ReadonlySet<string> = new Set(["］", "》", "」", "』", "〕"]);

export function registerSnippetTriggers(context: ExtensionContext): void {
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
    if (change.text.length !== 1) {
      return;
    }
    if (change.rangeLength > 1) {
      return;
    }

    busy = true;
    try {
      if (await maybeSkipOver(editor, event.document, change.text, change.range.start)) {
        return;
      }
      await maybeWrap(editor, event.document, change.text, change.range.start);
    } finally {
      busy = false;
    }
  };

  context.subscriptions.push(workspace.onDidChangeTextDocument((e) => void handler(e)));
}

async function maybeSkipOver(
  editor: import("vscode").TextEditor,
  doc: TextDocument,
  text: string,
  start: Position,
): Promise<boolean> {
  if (!SKIP_OVER_CHARS.has(text)) {
    return false;
  }
  const lineText = doc.lineAt(start.line).text;
  const nextCol = start.character + text.length;
  if (lineText.charAt(nextCol) !== text) {
    return false;
  }
  const version = doc.version;
  const deleteRange = new Range(start, new Position(start.line, nextCol));
  const ok = await editor.edit((eb) => eb.delete(deleteRange));
  if (!(ok && documentVersionMatches(version + 1, doc.version))) {
    return false;
  }
  const newPos = new Position(start.line, start.character + text.length);
  editor.selection = new Selection(newPos, newPos);
  return true;
}

async function maybeWrap(
  editor: import("vscode").TextEditor,
  doc: TextDocument,
  text: string,
  start: Position,
): Promise<void> {
  const rule = WRAPS.find((r) => r.trigger === text);
  if (!rule) {
    return;
  }
  if (text === "#" && isAfterOpenerBracket(doc, start)) {
    return;
  }
  if (rule.suppressIfNextIs !== undefined) {
    const lineText = doc.lineAt(start.line).text;
    const nextCol = start.character + text.length;
    if (lineText.charAt(nextCol) === rule.suppressIfNextIs) {
      return;
    }
  }
  const version = doc.version;
  const replaceRange = new Range(start, new Position(start.line, start.character + text.length));
  const ok = await editor.insertSnippet(new SnippetString(rule.body), replaceRange);
  if (!ok) {
    return;
  }
  if (
    rule.postExpandSuggest === true &&
    window.activeTextEditor === editor &&
    editor.document === doc &&
    documentVersionMatches(version + 1, doc.version)
  ) {
    await commands.executeCommand("editor.action.triggerSuggest");
  }
}

function isAfterOpenerBracket(doc: TextDocument, position: Position): boolean {
  if (position.character === 0) {
    return false;
  }
  const lineText = doc.lineAt(position.line).text;
  const prev = lineText.charAt(position.character - 1);
  return prev === "[" || prev === "［";
}
