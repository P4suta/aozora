import { describe, expect, it } from 'vitest';
import './__setup__/localStoragePolyfill';
import {
  buildShareUrl,
  compressedParamToText,
  paramToText,
  SHARE_URL_LIMIT,
  textToCompressedParam,
  textToParam,
} from '../share';

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

    it('高エントロピーな長文では tooLong: true（上限超）', () => {
      // 反復は ?c= 圧縮で潰れるので、圧縮の効かない高エントロピー文字列
      // （連続する別々の CJK 符号位置）で上限超過を確かめる。
      const long = Array.from({ length: SHARE_URL_LIMIT }, (_, i) =>
        String.fromCharCode(0x4e00 + (i % 0x3000)),
      ).join('');
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

  describe('圧縮共有 (textToCompressedParam / compressedParamToText)', () => {
    it('日本語を正しく往復する', () => {
      const input = '青空文庫記法のテスト。｜青梅《おうめ》';
      expect(compressedParamToText(textToCompressedParam(input))).toBe(input);
    });

    it('改行・絵文字・全角記号を往復する', () => {
      const input = '本📚を読む\n［＃「青梅」に傍点］\nline3';
      expect(compressedParamToText(textToCompressedParam(input))).toBe(input);
    });

    it('base64url 形式（+ / = を含まない）で出力される', () => {
      const input = '反復反復反復反復反復反復反復反復反復反復反復反復';
      expect(textToCompressedParam(input)).not.toMatch(/[+/=]/);
    });

    it('壊れた圧縮入力では例外を投げる', () => {
      expect(() => compressedParamToText('!!!')).toThrow();
    });

    it('反復の多い長文では圧縮が生 base64url より短い', () => {
      const long = 'あいうえお'.repeat(400);
      expect(textToCompressedParam(long).length).toBeLessThan(textToParam(long).length);
    });
  });

  describe('buildShareUrl のパラメータ選択 + 保全', () => {
    it('反復の多い長文では圧縮 ?c= を使う（上限内）', () => {
      const long = 'あいうえお'.repeat(200);
      const { url, tooLong } = buildShareUrl(long);
      expect(tooLong).toBe(false);
      expect(url).toMatch(/[?&]c=/);
      expect(url).not.toMatch(/[?&]text=/);
    });

    it('既存の ?lang= 等のクエリを保全する（query clobber しない）', () => {
      const orig = globalThis.location;
      Object.defineProperty(globalThis, 'location', {
        configurable: true,
        value: new URL('https://test.local/aozora/playground/?lang=en'),
      });
      try {
        const { url } = buildShareUrl('hello');
        expect(url).toContain('lang=en');
        expect(url).toMatch(/[?&]text=/);
      } finally {
        Object.defineProperty(globalThis, 'location', { configurable: true, value: orig });
      }
    });
  });
});
