# Aozora Bunko specification snapshot

Archived from <https://www.aozora.gr.jp/> on **2026-04-23** for offline
reference during aozora development. Re-run the fetch loop in this
directory to refresh.

| File | Section | Upstream URL |
|---|---|---|
| `accent_separation.html` | Accent decomposition (ASCII digraph → Unicode) | <https://www.aozora.gr.jp/accent_separation.html> |
| `annotation-index.html` | Annotation index / table of contents | <https://www.aozora.gr.jp/annotation/index.html> |
| `annotation-layout_1.html` | Page breaks (改丁, 改ページ, 改見開き, 改段) | <https://www.aozora.gr.jp/annotation/layout_1.html> |
| `annotation-layout_2.html` | Indentation (字下げ, 地付き, 地寄せ, 字詰め complex) | <https://www.aozora.gr.jp/annotation/layout_2.html> |
| `annotation-layout_3.html` | Alignment (左右中央) | <https://www.aozora.gr.jp/annotation/layout_3.html> |
| `annotation-heading.html` | Headings (大/中/小 見出し, 窓, 目次) | <https://www.aozora.gr.jp/annotation/heading.html> |
| `annotation-external_character.html` | Gaiji (JIS X 0213, Unicode, accented Latin) | <https://www.aozora.gr.jp/annotation/external_character.html> |
| `annotation-kunten.html` | Kunten (返り点, 再読文字) | <https://www.aozora.gr.jp/annotation/kunten.html> |
| `annotation-emphasis.html` | Emphasis (傍点 all types, 傍線) | <https://www.aozora.gr.jp/annotation/emphasis.html> |
| `annotation-graphics.html` | Graphics (画像, キャプション) | <https://www.aozora.gr.jp/annotation/graphics.html> |
| `annotation-etc.html` | Misc (ルビ, 縦中横, 割り注, 罫囲み, 横組み, 文字サイズ) | <https://www.aozora.gr.jp/annotation/etc.html> |

## Why vendored

Aozora Bunko encoding / annotation conventions are subtle (see memory note
`feedback_read_aozora_spec_first`). A spec-first response to parser
surprises is only fast when the authoritative source is one `Read` tool
call away. Shipping these pages in-tree also means CI and offline dev
iterations never wait on the network.

## License / copyright

Quoted material is © Aozora Bunko volunteers and reproduced here verbatim
for technical reference under fair use. The text is small, stable, and
central to an interoperability effort. If the rights-holder requests
removal, drop the files and fall back to `WebFetch`.
