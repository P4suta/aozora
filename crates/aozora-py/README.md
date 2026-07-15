# aozora

Python bindings for [aozora](https://github.com/P4suta/aozora), a parser
for 青空文庫記法 (Aozora Bunko notation).

```sh
pip install aozora
```

```python
import aozora

aozora.Document("｜青梅《おうめ》の街").to_html()
```

Real 青空文庫 archive files are Shift_JIS, not UTF-8. Feed those in as
bytes and the encoding is detected for you:

```python
aozora.Document.from_bytes(open("kokoro.txt", "rb").read()).to_html()
```

The package is typed, and every method carries its documentation, so
`help(aozora.Document)` is the full surface.

- [Source & issues](https://github.com/P4suta/aozora)
- [The notation](https://p4suta.github.io/aozora-notation-spec/)

Apache-2.0 OR MIT, at your option.
