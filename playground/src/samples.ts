export interface Sample {
  id: string;
  title: string;
  text: string;
}

export const SAMPLES: Sample[] = [
  {
    id: 'ruby',
    title: 'ルビ（明示）',
    text: '｜青梅《おうめ》街道を歩く。',
  },
  {
    id: 'ruby-implicit',
    title: '暗黙ルビ',
    text: '青梅《おうめ》という地名。',
  },
  {
    id: 'bouten',
    title: '傍点',
    text: '青空［＃「青空」に傍点］を見上げる。',
  },
  {
    id: 'ruby-bouten',
    title: 'ルビ＋傍点',
    text: '｜青梅《おうめ》には［＃「青梅」に傍点］という街道がある。',
  },
  {
    id: 'indent',
    title: '字下げブロック',
    text: '本文。\n［＃ここから2字下げ］\n段落の中身。\n別の行。\n［＃ここで字下げ終わり］\n通常段落。',
  },
  {
    id: 'gaiji',
    title: '外字（第3水準）',
    text: '珍しき木※［＃「木＋吶のつくり」、第3水準1-85-54］が立つ。',
  },
  {
    id: 'angle-quote',
    title: '二重山括弧',
    text: '≪重要≫な点について述べる。',
  },
  {
    id: 'page-break',
    title: '改ページ',
    text: '第一章\n本文。\n［＃改ページ］\n第二章\n続き。',
  },
  {
    id: 'kitchen-sink',
    title: '全部のせ',
    text: '｜山《やま》や［＃改ページ］\n≪秘密≫の話題。\n青空［＃「青空」に傍点］を見上げる。\n［＃ここから1字下げ］\n字下げされた段落。\n［＃ここで字下げ終わり］',
  },
];

export const DEFAULT_SAMPLE_ID = 'ruby-bouten';
