// Shared notation styling for the preview pane and the HTML export.
//
// The renderer emits only `aozora-*` class hooks and ships a canonical
// reference stylesheet (crates/aozora-render/assets/aozora-notation.css,
// see ADR-0024); esbuild inlines it as a string (see esbuild.mjs). We
// consume that single source of truth here instead of hand-rolling
// `.aozora-*` rules — the old hand-rolled copies had drifted to dead
// class names (`aozora_gaiji`, `aozora_tcy`) so gaiji highlighting and
// 縦中横 both silently broke.

import notationCss from "../../../crates/aozora-render/assets/aozora-notation.css";

/**
 * The canonical notation stylesheet plus a theme bridge, ready to drop
 * into a `<style>` block. Consumers apply `.aozora-notation` (and
 * `.aozora-vertical` for 縦書き) to the container.
 *
 * The bridge maps the sheet's `--aozora-*` hooks onto VS Code theme
 * colours so the live preview follows the editor's light/dark theme;
 * each mapping keeps a literal fallback, so the same string also renders
 * correctly in the standalone HTML export (opened outside VS Code, where
 * the `--vscode-*` variables are undefined). The gaiji highlight keeps
 * the sheet's self-contained yellow, which reads on any theme.
 */
export const aozoraNotationStyles = `${notationCss}

.aozora-notation {
  --aozora-fg: var(--vscode-editor-foreground, #222);
  --aozora-muted: var(--vscode-descriptionForeground, #6b6b6b);
  --aozora-accent: var(--vscode-textLink-foreground, #0b66c3);
  --aozora-accent-bg: var(--vscode-textBlockQuote-background, rgba(11, 102, 195, 0.12));
  --aozora-border: var(--vscode-panel-border, #c8c8c8);
}`;
