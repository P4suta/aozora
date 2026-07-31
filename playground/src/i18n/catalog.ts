import type { Locale } from './types';

const ja = {
  editorPaneTitle: '入力（青空文庫記法）',
  editorPlaceholder: '青空文庫記法を入力…',
  hoverUnresolved: '（未解決）',
  compAnn: '＃ 注記',
  compAnnDetail: '［＃...］ 一行注記のテンプレート',
  compRuby: '｜ ルビ（明示）',
  compRubyDetail: '｜base《reading》 で明示ルビ',
  compImplicitRuby: '《 ルビ（暗黙）',
  compImplicitRubyDetail: '直前の漢字に読みを振る',
  compGaiji: '※ 外字',
  compGaijiDetail: '※［＃「desc」、mencode］',
  lintUnclosed: '括弧が閉じられていません',
  lintUnmatched: '対応する開き括弧がありません',
  lintPua: '私用領域文字が含まれています（{hex}）',
  lintStrayMarker: '分類できない注記が残っています',
};

export type MessageKey = keyof typeof ja;

const en: Record<MessageKey, string> = {
  editorPaneTitle: 'Aozora notation source',
  editorPlaceholder: 'Type Aozora notation…',
  hoverUnresolved: '(unresolved)',
  compAnn: '＃ Annotation',
  compAnnDetail: '［＃...］ one-line annotation template',
  compRuby: '｜ Ruby (explicit)',
  compRubyDetail: 'Explicit ruby via ｜base《reading》',
  compImplicitRuby: '《 Ruby (implicit)',
  compImplicitRubyDetail: 'Add a reading to the preceding kanji',
  compGaiji: '※ Gaiji',
  compGaijiDetail: '※［＃「desc」、mencode］',
  lintUnclosed: 'Unclosed bracket',
  lintUnmatched: 'No matching open bracket',
  lintPua: 'Contains a private-use character ({hex})',
  lintStrayMarker: 'An annotation marker could not be classified',
};

export const CATALOG: Record<Locale, Record<MessageKey, string>> = { ja, en };
