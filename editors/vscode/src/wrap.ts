// Selection-wrap commands — "select something, then add notation
// around it" actions, named after the typesetter's intent rather
// than the implementation shape.
//
// ## Design
//
// Each entry in [`WRAP_SHAPES`] is a typed record describing one
// command. The `template` is a snippet body where the literal token
// `BASE` is replaced by the (snippet-escaped) selected text and
// `$0` marks where the cursor lands after insertion. The shape is
// declared `as const` so the registered command IDs and templates
// flow into the rest of the file as string-literal types — typos
// become compile errors instead of "command not found at runtime".
//
// The LSP `code_action` path in
// `crates/aozora-lsp/src/code_actions.rs` mirrors these shapes for
// non-VS Code clients (helix, neovim).

import {
  commands,
  type ExtensionContext,
  Range,
  type Selection,
  SnippetString,
  type TextEditor,
  window,
} from "vscode";

/**
 * One wrap action. The template body uses two placeholders:
 *
 *   - `BASE` — replaced (literally — case-sensitive) by the
 *     snippet-escaped selected text. Multiple `BASE` occurrences in
 *     one template are all substituted (e.g. forward-bouten repeats
 *     the selection inside the annotation).
 *   - `$0`   — final cursor position after the snippet expands.
 *     Standard VS Code snippet syntax.
 *
 * Templates are checked at module load via the `satisfies` operator
 * below so a missing `$0` or `BASE` token is a *type error*, not a
 * silent UX bug.
 */
type WrapTemplate = `${string}${"BASE" | "$0"}${string}`;

interface WrapShape {
  /** Command ID registered with VS Code. Mirrored in `package.json`. */
  readonly id: `aozora.wrap.${string}`;
  /** Snippet body — see [`WrapTemplate`] for placeholder semantics. */
  readonly template: WrapTemplate;
}

const WRAP_SHAPES = [
  // Most common typesetter action: kanji selected, attach reading.
  // The leading `｜` is always inserted — aozora style guides
  // recommend it unconditionally because it pins the base's start
  // even when surrounding context would otherwise be ambiguous.
  // We picked the safe default rather than offering a "no-pipe"
  // variant because the no-pipe form is *only* equivalent when the
  // preceding char is non-kanji, and surfacing two near-identical
  // commands invited the user to pick the wrong one.
  { id: "aozora.wrap.ruby", template: "｜BASE《$0》" },
  // 二重ルビ for emphasis; rare but supported. Same pipe rule.
  { id: "aozora.wrap.doubleRuby", template: "｜BASE《《$0》》" },
  // 傍点 forward-reference: selection stays plain, the bouten note
  // appears after it with the same text repeated as the target.
  { id: "aozora.wrap.bouten", template: "BASE［＃「BASE」に傍点］$0" },
  // Traditional surrounds — selection ends up inside the brackets.
  { id: "aozora.wrap.kagikakko", template: "「BASE」$0" },
  { id: "aozora.wrap.kikkou", template: "〔BASE〕$0" },
  { id: "aozora.wrap.chuki", template: "［＃BASE］$0" },
] as const satisfies ReadonlyArray<WrapShape>;

/** Union of every registered wrap command ID — exported for tests
 *  and any future programmatic dispatcher. */
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
  // `insertSnippet` accepts one location at a time; loop selections
  // so multi-cursor users still get one wrap per cursor.
  for (const sel of targets) {
    await applyOne(editor, shape, sel);
  }
}

async function applyOne(editor: TextEditor, shape: WrapShape, sel: Selection): Promise<void> {
  const range = new Range(sel.start, sel.end);
  const body = expandTemplate(shape.template, editor.document.getText(range));
  await editor.insertSnippet(new SnippetString(body), range);
}

/**
 * Substitute every `BASE` in `template` with the snippet-escaped
 * `selected` text. Snippet metacharacters in the selection (`$`,
 * `}`, `\`) are escaped so user content can't subvert the snippet
 * grammar (e.g. a literal `$1` in the selection wouldn't introduce
 * a bogus tabstop).
 */
function expandTemplate(template: WrapTemplate, selected: string): string {
  return template.split("BASE").join(escapeForSnippet(selected));
}

function escapeForSnippet(text: string): string {
  return text.replace(/\\/g, "\\\\").replace(/\$/g, "\\$").replace(/\}/g, "\\}");
}
