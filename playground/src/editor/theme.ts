import { EditorView } from '@codemirror/view';

export const aozoraTheme = EditorView.theme({
  '&': {
    height: '100%',
    fontSize: '14px',
    background: 'Canvas',
    color: 'CanvasText',
  },
  '@media (max-width: 767px)': {
    '&': { fontSize: '16px' },
    '.cm-scroller': { lineHeight: '1.8' },
  },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': {
    fontFamily: "ui-monospace, 'SFMono-Regular', Menlo, Consolas, monospace",
    lineHeight: '1.75',
  },
  '.cm-content': {
    padding: '0.75rem 0',
    caretColor: 'CanvasText',
  },
  '.cm-gutters': {
    background: 'Canvas',
    color: 'GrayText',
    borderRight: '1px solid color-mix(in srgb, CanvasText 20%, transparent)',
  },
  '.cm-activeLine, .cm-activeLineGutter': {
    background: 'color-mix(in srgb, Highlight 12%, transparent)',
  },
  '.cm-selectionBackground, .cm-content ::selection': {
    background: 'Highlight',
    color: 'HighlightText',
  },
  '&.cm-focused .cm-cursor': { borderLeftColor: 'CanvasText' },
  '.cm-tooltip': {
    background: 'Canvas',
    color: 'CanvasText',
    border: '1px solid color-mix(in srgb, CanvasText 24%, transparent)',
  },
  '.cm-tooltip-aozora-gaiji': {
    padding: '0.5rem 0.75rem',
    maxWidth: '320px',
    fontSize: '13px',
    lineHeight: '1.55',
  },
  '.cm-tooltip-aozora-gaiji strong, .cm-aozora-ruby, .cm-aozora-angle-quote': {
    color: 'LinkText',
    fontWeight: '700',
  },
  '.cm-tooltip-aozora-gaiji .muted, .cm-aozora-directive, .cm-aozora-heading-hint, .cm-aozora-container-marker':
    {
      color: 'GrayText',
      fontStyle: 'italic',
    },
  '.cm-aozora-bouten': {
    textDecoration: 'underline dotted',
    fontStyle: 'italic',
  },
  '.cm-aozora-gaiji, .cm-aozora-inlay': {
    background: 'Mark',
    color: 'MarkText',
  },
  '.cm-aozora-gaiji': {
    borderBottom: '1px dotted currentColor',
  },
  '.cm-aozora-combine-upright': { textDecoration: 'underline dotted' },
  '.cm-aozora-illustration': { fontStyle: 'italic' },
  '.cm-aozora-warichu, .cm-aozora-kaeriten': {
    fontSize: '0.9em',
    color: 'GrayText',
  },
  '.cm-aozora-kaeriten': { verticalAlign: 'super' },
  '.cm-aozora-aozora-heading': {
    fontWeight: '700',
    textDecoration: 'underline',
  },
  '.cm-aozora-section-break, .cm-aozora-page-break': {
    background: 'color-mix(in srgb, CanvasText 8%, transparent)',
    color: 'GrayText',
  },
  '.cm-aozora-inlay': {
    borderRadius: '3px',
    padding: '0 0.25em',
    margin: '0 0.15em',
    fontSize: '0.9em',
    fontStyle: 'italic',
  },
});
