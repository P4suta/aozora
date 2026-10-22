// ADR-0024 shares the renderer style sheet to prevent preview and export styles from drifting.

import notationCss from "../../../crates/aozora/assets/aozora-notation.css";

/** Apply `.aozora-notation` and, for vertical writing, `.aozora-vertical` to the styled container. */
export const aozoraNotationStyles = `${notationCss}

.aozora-notation {
  --aozora-fg: var(--vscode-editor-foreground, #222);
  --aozora-muted: var(--vscode-descriptionForeground, #6b6b6b);
  --aozora-accent: var(--vscode-textLink-foreground, #0b66c3);
  --aozora-accent-bg: var(--vscode-textBlockQuote-background, rgba(11, 102, 195, 0.12));
  --aozora-border: var(--vscode-panel-border, #c8c8c8);
}`;
