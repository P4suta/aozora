import { beforeEach, describe, expect, it } from 'vitest';
import './__setup__/localStoragePolyfill';
import {
  clearStoredSource,
  loadNumber,
  loadStoredSource,
  loadString,
  removeKey,
  saveNumber,
  saveSource,
  saveString,
} from '../storage';

describe('storage.ts', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  describe('source storage', () => {
    it('保存と復元ができる', () => {
      expect(loadStoredSource()).toBeNull();
      expect(saveSource('｜青梅《おうめ》')).toBe(true);
      expect(loadStoredSource()).toBe('｜青梅《おうめ》');
    });

    it('空文字を保存すると key が消える', () => {
      saveSource('something');
      saveSource('');
      expect(loadStoredSource()).toBeNull();
    });

    it('clear で削除される', () => {
      saveSource('hello');
      clearStoredSource();
      expect(loadStoredSource()).toBeNull();
    });
  });

  describe('generic keyed helpers', () => {
    it('saveString / loadString が往復する', () => {
      expect(loadString('foo')).toBeNull();
      expect(saveString('foo', 'bar')).toBe(true);
      expect(loadString('foo')).toBe('bar');
    });

    it('saveNumber / loadNumber が往復する', () => {
      expect(loadNumber('count')).toBeNull();
      saveNumber('count', 42);
      expect(loadNumber('count')).toBe(42);
    });

    it('数値として解釈できない文字列は loadNumber が null を返す', () => {
      saveString('count', 'not-a-number');
      expect(loadNumber('count')).toBeNull();
    });

    it('removeKey で削除される', () => {
      saveString('foo', 'bar');
      removeKey('foo');
      expect(loadString('foo')).toBeNull();
    });

    it('key prefix を二重に付けない', () => {
      saveString('aozora-playground:already-prefixed', 'x');
      expect(loadString('aozora-playground:already-prefixed')).toBe('x');
      // raw localStorage に直接アクセスして prefix が 1 つだけか確認
      expect(localStorage.getItem('aozora-playground:already-prefixed')).toBe('x');
      expect(
        localStorage.getItem('aozora-playground:aozora-playground:already-prefixed'),
      ).toBeNull();
    });
  });
});
