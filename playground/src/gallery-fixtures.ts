import type { LocalizedText } from '@aozora/playground-ui';

export interface GalleryFixture {
  readonly family: string;
  readonly label: LocalizedText;
  readonly source: string;
}

export const GALLERY_FIXTURES: readonly GalleryFixture[] = [
  {
    family: 'ruby',
    label: { ja: 'ルビ', en: 'Ruby' },
    source: '悟浄《ごじょう》の肉体はもはや疲れ切っていた。',
  },
  {
    family: 'bouten',
    label: { ja: '傍点', en: 'Emphasis dots' },
    source: 'ふらんす［＃「ふらんす」に傍点］はあまりに遠し',
  },
  {
    family: 'tcy',
    label: { ja: '縦中横', en: 'Tate-chu-yoko' },
    source: '（10［＃「10」は縦中横］）「かいともし、とうよ」',
  },
  {
    family: 'kaeriten',
    label: { ja: '返り点', en: 'Kaeriten' },
    source: '漢文［＃上二］また［＃下二］。',
  },
  {
    family: 'gaiji',
    label: { ja: '外字', en: 'Gaiji' },
    source:
      '美女、瞳を※［＃「目＋爭」、第3水準1-88-85］《みは》る。未解決：※［＃「架空の外字」、第3水準99-99-99］',
  },
  {
    family: 'angle-quote',
    label: { ja: '二重山括弧', en: 'Double angle brackets' },
    source: '≪風は冷気をつつんでゐる≫',
  },
  {
    family: 'warichu',
    label: { ja: '割り注', en: 'Inline note' },
    source: '一、乳油［＃割り注］洋名バタ［＃割り注終わり］',
  },
];
