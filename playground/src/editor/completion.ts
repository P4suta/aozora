import {
  autocompletion,
  snippet,
  type Completion,
  type CompletionContext,
  type CompletionResult,
  type CompletionSource,
} from '@codemirror/autocomplete';
import type { EditorView } from '@codemirror/view';
import { loadSlugCatalog, type SlugEntry } from './slugCatalog';
import { t, type MessageKey } from '../i18n';

/**
 * Structured snippets — single-character triggers that immediately
 * expand into a parameterised template the user can tab through.
 *
 * 仕様メモ：
 * - すべて青空文庫記法の全角文字で構成する。半角を残さない
 * - trigger 文字も snippet 内に保持する（`｜` の前置や `※` のマーカーは
 *   記法上の意味があるので、accept してもユーザーが打った文字は消えない）
 * - `${1:placeholder}` で初期 selection、`${0}` で最終カーソル位置
 */
interface TriggerSnippet {
  trigger: string;
  snippet: string;
  labelKey: MessageKey;
  detailKey: MessageKey;
}

const TRIGGER_SNIPPETS: TriggerSnippet[] = [
  // ＃ → ［＃...］：1 行注記。onType で `[` から既に ［＃］ が入る場合の
  // 補完は slug カタログが担当するので、これは ＃ 単独で打った時のフォールバック
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
  // ｜ → ｜${base}《${reading}》：明示ルビ。trigger の ｜ を保持して
  // ${base} を最初に selection、Tab で reading に進む
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
  // 《 → 《${reading}》：直前 CJK 文字に読みを振る暗黙ルビ
  {
    trigger: '《',
    snippet: '《${1:reading}》',
    labelKey: 'compImplicitRuby',
    detailKey: 'compImplicitRubyDetail',
  },
  // ※ → ※［＃「${description}」、${mencode}］：外字テンプレート
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

/**
 * Slug 補完。`apply` を関数化して、accept 時に既存の `］` を検知して
 * 消費するロジックを入れる。onType filter が `[` から `［＃］` を
 * 挿入済みで cursor が `＃` と `］` の間にあるケースを綺麗に扱える。
 */
function slugCompletion(entry: SlugEntry): Completion {
  const body = entry.accepts_param
    ? entry.canonical.replace(/\{N\}/g, '${1:1}')
    : entry.canonical;

  // Block container open は close marker を別行に同時挿入する。
  // 内側に最終カーソル `${0}` を置く。
  const template =
    entry.family === 'blockContainerOpen' && entry.partner
      ? `${body}］\n\${0}\n［＃${entry.partner}］`
      : `${body}］\${0}`;

  return {
    label: entry.canonical,
    type: familyToKind(entry.family),
    detail: entry.doc,
    apply: (view: EditorView, completion: Completion, from: number, to: number) => {
      // 既存の `］`（onType が ［＃］ で挿入したペア）を消費する。
      // hasClosing=true なら範囲を `to + 1` まで広げて重複の `］` を防ぐ。
      const doc = view.state.doc;
      const after = doc.sliceString(to, Math.min(to + 1, doc.length));
      const hasClosing = after === '］';
      snippet(template)(view, completion, from, hasClosing ? to + 1 : to);
    },
  };
}

/**
 * Structured snippet を 1 件の補完候補として返す。trigger 自身は
 * snippet テンプレートに含めているので、置換範囲は trigger 1 文字を
 * 含む（から trigger 始点）→ context.pos まで。
 */
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

/**
 * `［＃` 直後で `＃` trigger の structured snippet を出すと redundant
 * （既に ［＃ が入っているのにさらに ［＃...］ を提案するのは謎）。
 * 直前 2 文字が `［＃` ならスキップする。
 */
function isInsideSlugBody(context: CompletionContext): boolean {
  if (context.pos < 2) return false;
  const before = context.state.sliceDoc(context.pos - 2, context.pos);
  return before === '［＃';
}

const aozoraCompletionSource: CompletionSource = (
  context: CompletionContext,
): CompletionResult | null => {
  // 1) スラグ補完: ［＃ もしくは [# の直後（カーソルが本体テキストにある間）
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

  // 2) Structured snippets: 直前 1 文字がトリガー
  for (const trig of TRIGGER_SNIPPETS) {
    if (!context.matchBefore(new RegExp(escapeRegex(trig.trigger) + '$'))) continue;
    // `＃` trigger は ［＃ 直後では出さない（slug カタログが優先）
    if ((trig.trigger === '＃' || trig.trigger === '#') && isInsideSlugBody(context)) {
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
