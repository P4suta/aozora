import { hoverTooltip, type Tooltip } from '@codemirror/view';
import { t } from '../i18n';
import {
  byteToUtf16,
  type ParserState,
  parserStateField,
  utf16ToByte,
} from './parserState';

function formatCodepoint(cp: number | undefined): string {
  if (cp === undefined) return '';
  return `U+${cp.toString(16).toUpperCase().padStart(4, '0')}`;
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/**
 * Hover tooltip for `※［＃...］` gaiji references.
 *
 * Delegates the actual resolution to the parsed WASM document.
 */
export const aozoraHover = hoverTooltip((view, pos): Tooltip | null => {
  const ps: ParserState = view.state.field(parserStateField);
  if (!ps.doc) return null;
  const byteOffset = utf16ToByte(ps, pos);
  const r = ps.doc.gaijiAt(byteOffset);
  if (!r) return null;
  const from = byteToUtf16(ps, r.span.start);
  const to = byteToUtf16(ps, r.span.end);
  return {
    pos: from,
    end: to,
    above: true,
    create() {
      const dom = document.createElement('div');
      dom.className = 'cm-tooltip-aozora-gaiji';
      const resolvedHtml = r.resolved
        ? `<strong>${escapeHtml(r.resolved)}</strong>`
        : `<span class="muted">${t('hoverUnresolved')}</span>`;
      const cp = formatCodepoint(r.codepoint);
      const cpHtml = cp ? ` <span class="muted">${cp}</span>` : '';
      const mencodeHtml = r.mencode
        ? `<br/><span class="muted">mencode: ${escapeHtml(r.mencode)}</span>`
        : '';
      dom.innerHTML = `${resolvedHtml}${cpHtml}<br/><span>${escapeHtml(r.description)}</span>${mencodeHtml}`;
      return { dom };
    },
  };
});
