export interface Sample {
  id: string;
  title: string;
  text: string;
  /** Provenance of the public-domain Aozora Bunko excerpt. */
  source: string;
}

/**
 * Demo samples for the playground. Each is a real, copyright-safe excerpt from
 * Aozora Bunko chosen to demonstrate one notation feature, verified to render with
 * zero diagnostics. Prefer real corpus text over invented strings so the demo
 * is both authentic and self-documenting.
 */
export const SAMPLES: Sample[] = [
  {
    id: 'ruby',
    title: 'ルビ（明示）',
    // The place name contains small katakana, so an explicit delimiter is required.
    text: '「先生｜雑司ヶ谷《ぞうしがや》の銀杏はもう散ってしまったでしょうか」',
    source: '夏目漱石『こころ』',
  },
  {
    id: 'ruby-implicit',
    title: '暗黙ルビ',
    text: '悟浄《ごじょう》の肉体はもはや疲れ切っていた。',
    source: '中島敦『悟浄出世』',
  },
  {
    id: 'bouten',
    title: '傍点',
    text: 'ふらんす［＃「ふらんす」に傍点］はあまりに遠し',
    source: '中島敦『十年』',
  },
  {
    id: 'ruby-bouten',
    title: 'ルビ＋傍点',
    text: '閑《しづか》さや岩にしみ入る［＃「しみ入る」に傍点］蝉の声',
    source: '芥川竜之介『芭蕉雑記』',
  },
  {
    id: 'indent',
    title: '字下げブロック',
    text: '［＃ここから２字下げ］\n花の頃を越えてかしこし馬に嫁\n［＃ここで字下げ終わり］',
    source: '夏目漱石『草枕』',
  },
  {
    id: 'gaiji',
    title: '外字（第3水準）',
    text: '美女、瞳を※［＃「目＋爭」、第3水準1-88-85］《みは》る。',
    source: '泉鏡花『海神別荘』',
  },
  {
    id: 'angle-quote',
    title: '二重山括弧',
    text: '≪風は冷気をつつんでゐる≫',
    source: '北条民雄『烙印をおされて』',
  },
  {
    id: 'page-break',
    title: '改ページ',
    text: '　マリヤンが聞いたら何というだろうか？\n［＃改ページ］\n　　　風物抄',
    source: '中島敦『環礁』',
  },
  {
    id: 'heading',
    title: '大見出し',
    text: '［＃ここから大見出し］\n夢と現実\n［＃ここで大見出し終わり］',
    source: '与謝野晶子『晶子詩篇全集』',
  },
  {
    id: 'tcy',
    title: '縦中横',
    text: '（10［＃「10」は縦中横］）「かいともし、とうよ」',
    source: '佐藤春夫『現代語訳 徒然草』',
  },
  {
    id: 'warichu',
    title: '割り注',
    text: '一、乳油［＃割り注］洋名バタ［＃割り注終わり］',
    source: '福沢諭吉『肉食之説』',
  },
  {
    id: 'keigakomi',
    title: '罫囲み',
    text: '［＃ここから罫囲み］\n　今夜、ほうせきをもらいに行く。いくら用心しても、だめだよ。二十めんそう\n［＃ここで罫囲み終わり］',
    source: '江戸川乱歩『ふしぎな人』',
  },
  {
    id: 'bousen',
    title: '傍線',
    text: '虚子ハ男子ヲ挙ゲタ。僕ガ年尾［＃「年尾」に傍線］トツケテヤッタ。',
    source: '夏目漱石『吾輩は猫である』中篇自序',
  },
  {
    id: 'jitsuki',
    title: '地付き',
    text: '［＃地付き］――Ｊ・Ｂ・ステェア「サモア地誌」――',
    source: '中島敦『光と風と夢』',
  },
  {
    id: 'kitchen-sink',
    title: '全部のせ',
    text: '［＃２字下げ］お猿［＃「お猿」は中見出し］\n\nお猿が出て来た、\n負はれて出て来た。\nお目をぱちくり［＃「ぱちくり」に傍点］、\n赤ん坊《ぼ》のお猿。',
    source: '与謝野晶子『晶子詩篇全集拾遺』',
  },
  {
    id: 'long-form',
    title: '長文（山月記・性能計測向け）',
    text: `［＃ここから大見出し］
山月記
［＃ここで大見出し終わり］

　隴西《ろうさい》の李徴《りちょう》は博学｜才穎《さいえい》、天宝の末年、若くして名を虎榜《こぼう》に連ね、ついで江南尉《こうなんい》に補せられたが、性、狷介《けんかい》、自《みずか》ら恃《たの》むところ頗《すこぶ》る厚く、賤吏《せんり》に甘んずるを潔《いさぎよ》しとしなかった。

　いくばくもなく官を退いた後は、故山、虢略《かくりゃく》に帰臥《きが》し、人と交を絶って、ひたすら詩作に耽《ふけ》った。

［＃ここから2字下げ］
　下吏となって長く膝を屈するよりは、詩家としての名を死後百年に遺《のこ》そうとしたのである。
［＃ここで字下げ終わり］

　しかし、文名は容易に揚《あが》らず、生活は日を逐《お》うて苦しくなる。李徴はようやく｜焦躁《しょうそう》に駆られて来た［＃「焦躁」に傍点］。`,
    source: '中島敦『山月記』',
  },
];

export const DEFAULT_SAMPLE_ID = 'angle-quote';
