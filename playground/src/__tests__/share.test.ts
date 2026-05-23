import { describe, expect, it } from 'vitest';
import './__setup__/localStoragePolyfill';
import { buildShareUrl, paramToText, SHARE_URL_LIMIT, textToParam } from '../share';

describe('share.ts', () => {
  describe('textToParam / paramToText 往復', () => {
    it('ASCII テキストを正しくエンコード・デコードする', () => {
      const input = 'hello world';
      const encoded = textToParam(input);
      expect(paramToText(encoded)).toBe(input);
    });

    it('日本語（UTF-8 multi-byte）を正しく往復する', () => {
      const input = '青空文庫記法のテスト';
      const encoded = textToParam(input);
      expect(paramToText(encoded)).toBe(input);
    });

    it('全角括弧と記号を正しく往復する', () => {
      const input = '｜青梅《おうめ》には［＃「青梅」に傍点］';
      const encoded = textToParam(input);
      expect(paramToText(encoded)).toBe(input);
    });

    it('改行コードを保存する', () => {
      const input = 'line1\nline2\nline3';
      const encoded = textToParam(input);
      expect(paramToText(encoded)).toBe(input);
    });

    it('絵文字（surrogate pair）を正しく往復する', () => {
      const input = '本📚を読む';
      const encoded = textToParam(input);
      expect(paramToText(encoded)).toBe(input);
    });

    it('空文字を往復できる', () => {
      expect(paramToText(textToParam(''))).toBe('');
    });

    it('base64url 形式（+ や / を含まない）で出力される', () => {
      const input = 'たくさんの日本語テキストをエンコードして長くする！？';
      const encoded = textToParam(input);
      expect(encoded).not.toMatch(/[+/=]/);
    });
  });

  describe('buildShareUrl', () => {
    it('短いテキストでは tooLong: false', () => {
      const res = buildShareUrl('hello');
      expect(res.tooLong).toBe(false);
      expect(res.url).toMatch(/\?text=/);
    });

    it('長文では tooLong: true（上限 3500 文字超）', () => {
      // 単純な ASCII を十分に長く積む。base64url 化は ~4/3 倍に膨らむ
      const long = 'a'.repeat(SHARE_URL_LIMIT * 2);
      const res = buildShareUrl(long);
      expect(res.tooLong).toBe(true);
    });
  });

  describe('paramToText の異常系', () => {
    it('壊れた base64 入力では例外が投げられる', () => {
      // 'ZZ' は base64 として valid だが atob/btoa で問題なく動く。
      // 不正文字 '!' を含むものを試す。
      expect(() => paramToText('!!!')).toThrow();
    });
  });
});
