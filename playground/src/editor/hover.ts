import { hoverTooltip, type Tooltip } from '@codemirror/view';
import { byteToUtf16, parserStateField, utf16ToByte, type ParserState } from './parserState';

interface GaijiResolution {
  span: { start: number; end: number };
  description: string;
  mencode: string | null;
  codepoint: number | null;
  resolved: string | null;
}

function formatCodepoint(cp: number | null): string {
  if (cp === null || cp === undefined) return '';
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
 * Delegates the actual resolution to aozora-wasm
 * (`Document.resolveGaijiAt`), which scans a 512-byte window
 * around the byte offset and returns either `"null"` (not in a
 * gaiji span) or a JSON object with span/description/mencode/
 * codepoint/resolved.
 */
export const aozoraHover = hoverTooltip((view, pos): Tooltip | null => {
  const ps: ParserState = view.state.field(parserStateField);
  if (!ps.doc) return null;
  const byteOffset = utf16ToByte(ps, pos);
  const json = ps.doc.resolveGaijiAt(byteOffset);
  if (!json || json === 'null') return null;
  let r: GaijiResolution;
  try {
    r = JSON.parse(json) as GaijiResolution;
  } catch {
    return null;
  }
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
        : `<span class="muted">(未解決)</span>`;
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
