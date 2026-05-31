import { EditorView } from '@codemirror/view';

/**
 * CM6 theme that ties into the playground's existing `:root` CSS
 * variables so the editor follows light/dark mode in sync with the
 * surrounding shell.
 *
 * The `.cm-aozora-*` token classes are emitted by
 * `decorations.ts` based on `nodes_json` from the WASM parser.
 */
export const aozoraTheme = EditorView.theme({
  '&': {
    height: '100%',
    fontSize: '14px',
    background: 'var(--bg-elev)',
    color: 'var(--fg)',
  },
  // Bump font-size on narrow viewports so iOS Safari does not zoom
  // the page when focusing the editor (any input < 16px triggers the
  // auto-zoom). Also helps readability on small phones.
  '@media (max-width: 760px)': {
    '&': { fontSize: '16px' },
    '.cm-scroller': { lineHeight: '1.8' },
  },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': {
    fontFamily:
      'var(--font-mono)',
    lineHeight: '1.75',
  },
  '.cm-content': {
    padding: '0.75rem 0',
    caretColor: 'var(--accent)',
  },
  '.cm-gutters': {
    background: 'var(--bg)',
    color: 'var(--fg-muted)',
    borderRight: '1px solid var(--border)',
  },
  '.cm-activeLine': { background: 'var(--accent-bg)' },
  '.cm-activeLineGutter': { background: 'var(--accent-bg)' },
  '.cm-selectionBackground, .cm-content ::selection': {
    background: 'var(--accent-selection)',
  },
  '&.cm-focused .cm-cursor': { borderLeftColor: 'var(--accent)' },
  '.cm-tooltip': {
    background: 'var(--bg-elev)',
    color: 'var(--fg)',
    border: '1px solid var(--border)',
    borderRadius: '4px',
    boxShadow: '0 4px 12px rgba(0, 0, 0, 0.12)',
  },
  '.cm-tooltip-aozora-gaiji': {
    padding: '0.5rem 0.75rem',
    maxWidth: '320px',
    fontSize: '13px',
    lineHeight: '1.55',
  },
  '.cm-tooltip-aozora-gaiji strong': {
    color: 'var(--token-gaiji)',
  },
  '.cm-tooltip-aozora-gaiji .muted': {
    color: 'var(--fg-muted)',
    fontFamily: "'Menlo', 'Consolas', monospace",
    fontSize: '0.85em',
  },

  // ===== nodes_json kind markers =====
  '.cm-aozora-ruby': {
    color: 'var(--token-ruby)',
    fontWeight: '600',
  },
  '.cm-aozora-double-ruby': {
    color: 'var(--token-ruby)',
    fontWeight: '700',
    background: 'var(--accent-bg)',
    borderRadius: '2px',
  },
  '.cm-aozora-bouten': {
    color: 'var(--token-bouten)',
    background: 'var(--token-bouten-bg)',
    fontStyle: 'italic',
  },
  '.cm-aozora-gaiji': {
    color: 'var(--token-gaiji)',
    background: 'var(--token-gaiji-bg)',
    borderBottom: '1px dotted var(--token-gaiji-border)',
  },
  '.cm-aozora-tcy': { textDecoration: 'underline dotted' },
  '.cm-aozora-sashie': { color: 'var(--success)', fontStyle: 'italic' },
  '.cm-aozora-warichu': {
    fontSize: '0.9em',
    color: 'var(--fg-muted)',
    background: 'rgba(160, 160, 170, 0.08)',
  },
  '.cm-aozora-kaeriten': {
    fontSize: '0.85em',
    color: 'var(--accent)',
    verticalAlign: 'super',
  },
  '.cm-aozora-annotation': { color: 'var(--fg-muted)', fontStyle: 'italic' },
  '.cm-aozora-aozora-heading': {
    color: 'var(--accent)',
    fontWeight: '700',
    textDecoration: 'underline',
  },
  '.cm-aozora-heading-hint': {
    color: 'var(--fg-muted)',
    fontStyle: 'italic',
  },
  '.cm-aozora-section-break': {
    color: 'var(--fg-muted)',
    background: 'rgba(160, 160, 170, 0.08)',
    fontWeight: '500',
  },
  '.cm-aozora-page-break': {
    color: 'var(--fg-muted)',
    background: 'rgba(160, 160, 170, 0.12)',
    fontWeight: '500',
  },
  '.cm-aozora-container-marker': {
    color: 'var(--fg-muted)',
    fontStyle: 'italic',
    fontWeight: '500',
  },

  // ===== gaiji inlay (Section 13) =====
  '.cm-aozora-inlay': {
    color: 'var(--token-gaiji)',
    background: 'var(--token-gaiji-bg)',
    borderRadius: '3px',
    padding: '0 0.25em',
    margin: '0 0.15em',
    fontSize: '0.9em',
    fontStyle: 'italic',
  },
});
