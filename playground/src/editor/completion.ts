import {
  autocompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult,
  type CompletionSource,
  snippet,
} from '@codemirror/autocomplete';
import type { EditorView } from '@codemirror/view';
import { type MessageKey, t } from '../i18n';
import { loadSlugCatalog, type SlugEntry } from './slugCatalog';

interface TriggerSnippet {
  trigger: string;
  snippet: string;
  labelKey: MessageKey;
  detailKey: MessageKey;
}

const TRIGGER_SNIPPETS: TriggerSnippet[] = [
  {
    trigger: '#',
    snippet: '［＃${1:body}］',
    labelKey: 'compAnn',
    detailKey: 'compAnnDetail',
  },
  {
    trigger: '＃',
    snippet: '［＃${1:body}］',
    labelKey: 'compAnn',
    detailKey: 'compAnnDetail',
  },
  {
    trigger: '|',
    snippet: '｜${1:base}《${2:reading}》',
    labelKey: 'compRuby',
    detailKey: 'compRubyDetail',
  },
  {
    trigger: '｜',
    snippet: '｜${1:base}《${2:reading}》',
    labelKey: 'compRuby',
    detailKey: 'compRubyDetail',
  },
  {
    trigger: '《',
    snippet: '《${1:reading}》',
    labelKey: 'compImplicitRuby',
    detailKey: 'compImplicitRubyDetail',
  },
  {
    trigger: '※',
    snippet: '※［＃「${1:description}」、${2:mencode}］',
    labelKey: 'compGaiji',
    detailKey: 'compGaijiDetail',
  },
];

/** Slug opener forms recognised both as full-width and half-width prefixes. */
const SLUG_OPENERS = ['［＃', '［#', '[＃', '[#'];

function familyToKind(family: string): Completion['type'] {
  switch (family) {
    case 'pageBreak':
    case 'section':
      return 'keyword';
    case 'blockContainerOpen':
    case 'blockContainerClose':
      return 'namespace';
    case 'leafAlign':
      return 'property';
    case 'bouten':
    case 'tateChuYoko':
    case 'warichu':
    case 'keigakomi':
      return 'function';
    case 'sashie':
      return 'class';
    case 'kaeritenSingle':
    case 'kaeritenCompound':
      return 'enum';
    default:
      return 'text';
  }
}

function slugCompletion(entry: SlugEntry): Completion {
  const body = entry.accepts_param
    ? entry.canonical.replace(/\{N\}/g, '${1:1}')
    : entry.canonical;

  const template =
    entry.family === 'blockContainerOpen' && entry.partner
      ? `${body}］\n\${0}\n［＃${entry.partner}］`
      : `${body}］\${0}`;

  return {
    label: entry.canonical,
    type: familyToKind(entry.family),
    detail: entry.doc,
    apply: (
      view: EditorView,
      completion: Completion,
      from: number,
      to: number,
    ) => {
      const doc = view.state.doc;
      const after = doc.sliceString(to, Math.min(to + 1, doc.length));
      const hasClosing = after === '］';
      snippet(template)(view, completion, from, hasClosing ? to + 1 : to);
    },
  };
}

function buildSnippetCompletion(trig: TriggerSnippet): Completion {
  return {
    label: t(trig.labelKey),
    type: 'snippet',
    detail: t(trig.detailKey),
    apply: snippet(trig.snippet),
  };
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function isInsideSlugBody(context: CompletionContext): boolean {
  if (context.pos < 2) return false;
  const before = context.state.sliceDoc(context.pos - 2, context.pos);
  return before === '［＃';
}

const aozoraCompletionSource: CompletionSource = (
  context: CompletionContext,
): CompletionResult | null => {
  for (const opener of SLUG_OPENERS) {
    const slugMatch = context.matchBefore(
      new RegExp(`${escapeRegex(opener)}([^］\\]\\n]*)$`),
    );
    if (slugMatch) {
      const slugs = loadSlugCatalog();
      const bodyStart = slugMatch.from + opener.length;
      return {
        from: bodyStart,
        to: context.pos,
        options: slugs.map(slugCompletion),
        validFor: /^[^］\]\n]*$/,
      };
    }
  }

  for (const trig of TRIGGER_SNIPPETS) {
    if (!context.matchBefore(new RegExp(`${escapeRegex(trig.trigger)}$`)))
      continue;
    if (
      (trig.trigger === '＃' || trig.trigger === '#') &&
      isInsideSlugBody(context)
    ) {
      continue;
    }
    return {
      from: context.pos - trig.trigger.length,
      to: context.pos,
      options: [buildSnippetCompletion(trig)],
      validFor: /^$/,
    };
  }

  return null;
};

export const aozoraCompletion = autocompletion({
  override: [aozoraCompletionSource],
  // Aozora notation has no whitespace-delimited words; the default
  // closeOnBlur=true is fine, but we make activate-on-typing snappy.
  activateOnTyping: true,
  defaultKeymap: true,
});
