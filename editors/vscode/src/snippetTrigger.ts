// `snippetTrigger.ts` — IDE-style aggressive auto-expansion + bracket
// pairing for the aozora notation entry chars.
//
// Why this lives in the VS Code extension and NOT in the LSP server:
//
// LSP `textDocument/onTypeFormatting` returns `TextEdit[]` only — no
// snippet placeholders, no cursor placement. Spec-correct for what
// the protocol covers, but it cannot deliver the IDE-style flow the
// user asked for (2026-04-30): type `｜`, get `｜<base>《<reading>》`
// with `<base>` selected, Tab to `<reading>`, Tab to escape; type
// `[`, get `［<cursor>］` (close auto-paired); type `]` next to an
// auto-paired `］` and have it skip-over instead of double-inserting.
//
// All of those need either snippet `${1:label}` placeholders + cursor
// placement (only `editor.insertSnippet` can do those) or programmatic
// cursor advance (only `editor.selection = ...` can do that). The LSP
// onType remains for the simple half-→full-width swaps, since
// non-VS-Code clients (helix / neovim) need it.
//
// ## Event ordering when LSP onType is in the mix
//
// The two paths chain cleanly:
//
//   user types `[` → LSP onType swaps to `［` → onDidChangeTextDocument
//   fires with text=`［` → we wrap to `［${0}］`
//
// We trigger on the **post-onType** form (`［`, `《`, `｜`, `※`),
// plus `#` which the LSP deliberately does NOT auto-convert (it
// appears in URLs and CSS hex). The `change.text` we see is the
// final, post-onType text.
//
// ## Skip-over for closing chars
//
// When the user types `]` (or `]`-via-LSP-onType `］`) and the char
// immediately ahead is already `］` (because we wrapped earlier),
// inserting another `］` would yield `［＃改ページ］］`. Standard
// IDE behavior is to detect this and SKIP-OVER the existing close
// instead of inserting a new one. VS Code's `autoClosingPairs`
// language-configuration setting does this for keystroke-typed
// chars but NOT for chars synthesised by our LSP onType, so we
// duplicate the logic here.
//
// ## Suppression
//
// `autoClosingPairs` may have already paired `［` typed directly via
// the user's IME with `］`. If we then wrap to `［${0}］`, we'd get
// `［${0}］］`. The suppression checks `the next char after the typed
// open` and bails if it's already the matching close.
//
// ## Re-entry guard
//
// `editor.insertSnippet` itself fires `onDidChangeTextDocument` for
// the snippet body. The `busy` flag swallows those re-entries.

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

/**
 * Wrap rules. The `trigger` is the just-typed char (post LSP onType).
 * `body` is the snippet expanded in place of the trigger; the trigger
 * char itself is part of the body (e.g. `［` rule's body starts with
 * `［`) so the splice is a clean single-step replacement.
 *
 * `suppressIfNextIs` skips the wrap when the char immediately after
 * the just-typed trigger is the listed char — that means VS Code's
 * `autoClosingPairs` already paired the open, so we'd otherwise
 * double-pair.
 */
interface WrapRule {
  trigger: string;
  body: string;
  suppressIfNextIs?: string;
  /** When true, fire `editor.action.triggerSuggest` after the snippet
   *  expansion so the LSP slug-catalogue popup appears at the new
   *  cursor position. Only meaningful for `#` today.
   */
  postExpandSuggest?: boolean;
}

// biome-ignore-start lint/suspicious/noTemplateCurlyInString: VS Code
// snippet placeholders (`${1:label}` / `${0}`) are not JS template
// strings — they're the snippet syntax `editor.insertSnippet` interprets.
const WRAPS: readonly WrapRule[] = [
  // Half-width `#` typed directly. The LSP onType deliberately does
  // not auto-convert `#`, so we get the literal here.
  {
    trigger: "#",
    body: "［＃${0}］",
    postExpandSuggest: true,
  },
  // Full-width forms — these arrive after LSP onType has converted
  // their half-width counterparts.
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
  // 亀甲括弧 — accent decomposition wrapper (e.g. `〔café〕`). The
  // LSP onType converts `{` to `〔`; we wrap to match the bracket-
  // pair behaviour `［…］` and `《…》` already provide.
  {
    trigger: "〔",
    body: "〔${0}〕",
    suppressIfNextIs: "〕",
  },
];
// biome-ignore-end lint/suspicious/noTemplateCurlyInString: see open comment above

/**
 * Closing chars that should skip-over the next char if it matches —
 * the standard IDE bracket-pair behavior, applied to chars LSP onType
 * just synthesised (which would otherwise miss VS Code's built-in
 * `autoClosingPairs` skip-over).
 */
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

    // Single-cursor single-char input only. Multi-cursor + paste are
    // out-of-scope (placeholder semantics ambiguous; pastes aren't
    // the IDE-flow case).
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
    // `rangeLength === 0` is a pure insert; `rangeLength === 1` is
    // LSP onType replacing one char with another (`|` → `｜`). Both
    // are "user effectively typed one new char." Bigger replacements
    // are skipped.
    if (change.rangeLength > 1) {
      return;
    }

    busy = true;
    try {
      // Skip-over has priority over wrap — typing `]` next to an
      // existing `］` should NOT first wrap and then race; it should
      // just consume the typed close and advance.
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

/**
 * If `text` is a closing char and the char immediately ahead matches,
 * delete the just-typed close and advance the cursor past the existing
 * one. Returns true when the skip-over fired.
 */
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
  // The just-typed char now sits at column `start.character`; the
  // candidate skip-over target is the next char.
  const nextCol = start.character + text.length;
  if (lineText.charAt(nextCol) !== text) {
    return false;
  }
  // Delete the just-typed close, then move cursor past the existing one.
  const version = doc.version;
  const deleteRange = new Range(start, new Position(start.line, nextCol));
  const ok = await editor.edit((eb) => eb.delete(deleteRange));
  if (!(ok && documentVersionMatches(version + 1, doc.version))) {
    return false;
  }
  // After deletion, the existing close moved into position `start.character`.
  // Place cursor immediately after it.
  const newPos = new Position(start.line, start.character + text.length);
  editor.selection = new Selection(newPos, newPos);
  return true;
}

/**
 * If `text` matches a wrap rule and suppression doesn't fire, splice
 * the wrap body in place via `editor.insertSnippet`. Returns void;
 * the operation is fire-and-forget from the caller's perspective.
 */
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
  // Suppress when an opener bracket already sits before the typed
  // char — that case belongs to the LSP slug-catalogue popup path.
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
  // Replace the just-typed trigger with the snippet body in one
  // undo step. `insertSnippet(SnippetString, Range)` does the splice
  // atomically.
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

/** True when the char immediately before `position` is `[` or `［`. */
function isAfterOpenerBracket(doc: TextDocument, position: Position): boolean {
  if (position.character === 0) {
    return false;
  }
  const lineText = doc.lineAt(position.line).text;
  const prev = lineText.charAt(position.character - 1);
  return prev === "[" || prev === "［";
}
