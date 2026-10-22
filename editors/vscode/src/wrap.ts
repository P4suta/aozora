import {
  commands,
  type ExtensionContext,
  Range,
  Selection,
  SnippetString,
  type TextEditor,
  window,
} from "vscode";

import { documentVersionMatches, expandWrapText, finalCursorOffsets } from "./extensionLogic";

type WrapTemplate = `${string}${"BASE" | "$0"}${string}`;

interface WrapShape {
  readonly id: `aozora.wrap.${string}`;
  readonly template: WrapTemplate;
}

const WRAP_SHAPES = [
  { id: "aozora.wrap.ruby", template: "｜BASE《$0》" },
  { id: "aozora.wrap.doubleRuby", template: "｜BASE《《$0》》" },
  { id: "aozora.wrap.bouten", template: "BASE［＃「BASE」に傍点］$0" },
  { id: "aozora.wrap.kagikakko", template: "「BASE」$0" },
  { id: "aozora.wrap.kikkou", template: "〔BASE〕$0" },
  { id: "aozora.wrap.chuki", template: "［＃BASE］$0" },
] as const satisfies ReadonlyArray<WrapShape>;

export type WrapCommandId = (typeof WRAP_SHAPES)[number]["id"];

export function registerWrapCommands(context: ExtensionContext): void {
  for (const shape of WRAP_SHAPES) {
    context.subscriptions.push(commands.registerCommand(shape.id, () => runWrap(shape)));
  }
}

async function runWrap(shape: WrapShape): Promise<void> {
  const editor = window.activeTextEditor;
  if (!editor) {
    void window.showInformationMessage("アクティブなエディタがありません。");
    return;
  }
  const targets = editor.selections.filter((s): s is Selection => !s.isEmpty);
  if (targets.length === 0) {
    void window.showInformationMessage("ルビをふる文字列を先にドラッグで選択してください。");
    return;
  }
  if (targets.length === 1) {
    const target = targets[0];
    if (target) {
      await applyOne(editor, shape, target);
    }
    return;
  }
  await applyMany(editor, shape, targets);
}

async function applyOne(editor: TextEditor, shape: WrapShape, sel: Selection): Promise<void> {
  const range = new Range(sel.start, sel.end);
  const body = expandTemplate(shape.template, editor.document.getText(range));
  await editor.insertSnippet(new SnippetString(body), range);
}

async function applyMany(
  editor: TextEditor,
  shape: WrapShape,
  selections: ReadonlyArray<Selection>,
): Promise<void> {
  const document = editor.document;
  const version = document.version;
  const replacements = selections.map((selection) => {
    const range = new Range(selection.start, selection.end);
    return {
      range,
      start: document.offsetAt(range.start),
      end: document.offsetAt(range.end),
      ...expandWrapText(shape.template, document.getText(range)),
    };
  });
  const cursorOffsets = finalCursorOffsets(replacements);
  const applied = await editor.edit((builder) => {
    for (const replacement of replacements) {
      builder.replace(replacement.range, replacement.text);
    }
  });
  if (!applied) {
    return;
  }
  if (editor.document !== document || !documentVersionMatches(version + 1, document.version)) {
    return;
  }
  editor.selections = cursorOffsets.map((offset) => {
    const position = document.positionAt(offset);
    return new Selection(position, position);
  });
}

function expandTemplate(template: WrapTemplate, selected: string): string {
  return template.split("BASE").join(escapeForSnippet(selected));
}

function escapeForSnippet(text: string): string {
  return text.replace(/\\/g, "\\\\").replace(/\$/g, "\\$").replace(/\}/g, "\\}");
}
