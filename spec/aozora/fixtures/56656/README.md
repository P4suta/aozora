# Golden Fixture: 青空文庫 Card 56656

**Work:** 罪と罰
**Author:** フョードル・ミハイロヴィチ・ドストエフスキー (Fyodor M. Dostoyevsky)
**Translator:** 米川正夫 (Masao Yonekawa)
**Aozora Bunko card:** <https://www.aozora.gr.jp/cards/001485/card56656.html>
**Copyright status:** Public domain (author d. 1881, translator d. 1966; Japan
translation copyright expired).

## Files

| File            | SHA256                                                             | Origin                                            |
|-----------------|--------------------------------------------------------------------|---------------------------------------------------|
| `input.zip`     | `2b54c2b8a87c5f129855c0213e4754cb3c82b95760803bdc5c0797e0b6456c42` | Aozora Bunko original download (`tsumito_batsu.txt` packed inside) |
| `input.sjis.txt`| `388819e28691b858e6503e5502ca1645d2d42f3c74dbdb10b4a3d5bbd6349591` | Unpacked `tsumito_batsu.txt`, Shift_JIS as published |
| `input.utf8.txt`| `64d189e6f160e9c86d5419a79e1735d31c9dbda609e763045946f75eeb4f6182` | UTF-8 re-encoding via `iconv` (preserved for parser tests) |
| `golden.html`   | `3b8b526b94e90c55950d7b0ac2516fdd4753b06647a241a13e3e07cb8a3bfc8b` | Aozora Bunko-generated reference HTML             |

## Why this text

Single-fixture coverage of the edge cases afm must handle for the 100% Aozora
compatibility promise:

- **1044 ruby annotations** — both explicit (`｜...《...》`) and implicit delimiter forms
- **Forward-reference bouten** `［＃「X」に傍点］` — bracket-after emphasis marker
- **Forward-reference headings** `［＃「第一篇」は大見出し］`
- **JIS X 0213 gaiji** `※［＃「木＋吶のつくり」、第3水準1-85-54］`
- **Accent decomposition** `〔Crevez chiens, si vous n'e^tes pas contents〕`
- **As-is (ママ) annotation** `［＃「」」はママ］`
- **Indentation** `［＃２字下げ］`, `［＃７字下げ］`
- Aozora header 凡例 block documenting the notations used

## M0 Spike acceptance

The goal at M0 Spike is for the parser to clear **Tier A**:

- zero panics
- zero unconsumed `［＃` sequences in the produced AST

Tier B (round-trip stability) and Tier C (diff-clean against `golden.html` modulo CSS
class differences) follow in M1 and M3 respectively.
