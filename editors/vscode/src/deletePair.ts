// `deletePair.ts` — auto-delete an empty bracket pair when the user
// deletes the last char inside it.
//
// User-facing scenario this addresses:
//
//   1. User types `#` → snippetTrigger expands to `［＃${0}］`.
//   2. User changes their mind and Backspaces the `＃`.
//   3. The natural expectation is that `［］` doesn't linger as
//      orphaned scaffolding — both brackets should also disappear.
//
// VS Code's built-in `editor.autoClosingDelete: "always"` (set via
// `configurationDefaults` for `[aozora]`) handles the *symmetric*
// case of "user backspaces the open bracket, also remove the close".
// What it does NOT handle is "user emptied the bracketed contents,
// also remove the brackets". This file is the missing half.
//
// ## Detection rule
//
// On every `onDidChangeTextDocument` event, we look for a single
// pure-deletion change (`text === ""`, `rangeLength >= 1`). After
// the deletion the cursor sits at `change.range.start`; if the
// chars at `cursor-1` and `cursor` form a matching open/close
// bracket pair from `BRACKET_PAIRS`, the deletion just emptied that
// pair and we splice both brackets out in one follow-up edit.
//
// The rule fires for EVERY adjacent matching pair from the language
// configuration's `autoClosingPairs` — `［…］`, `《…》`, `「…」`,
// `『…』`, `〔…〕`, `（…）`, `【…】`, `｛…｝`. Snippet-inserted pairs,
// LSP-formatted pairs, and pairs the user typed by hand all behave
// the same way; the user doesn't have to remember which is which.
//
// ## Re-entry guard
//
// Our follow-up edit is itself a deletion, which fires another
// `onDidChangeTextDocument`. The closure-scoped `busy` flag swallows
// the recursive event so we don't loop. The same pattern is used in
// `snippetTrigger.ts`; both guards are independent so neither blocks
// the other.

import {
  type ExtensionContext,
  Position,
  Range,
  type TextDocumentChangeEvent,
  window,
  workspace,
} from "vscode";

const LANG_ID = "aozora";

/**
 * Open → close map mirroring `language-configuration.json`'s
 * `autoClosingPairs`. Kept in sync by hand because the static
 * declaration there isn't accessible at runtime — VS Code applies
 * it but doesn't expose it back to the extension.
 */
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

    // Single pure-deletion only. Multi-cursor edits, replacements,
    // and inserts are out-of-scope (placeholder semantics get
    // ambiguous; the IDE-flow case is single-char Backspace/Delete).
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

    // After the deletion, the cursor occupies `change.range.start`.
    // The open candidate is the char immediately before that column,
    // the close candidate is the char now sitting at it. If they
    // form a known pair, the deletion just emptied that pair.
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
