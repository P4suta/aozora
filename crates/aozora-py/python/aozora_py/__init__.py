"""Aozora Bunko notation parser — Python bindings.

The public surface is the :class:`Document` class plus the module-level
helpers :func:`parse_to_html`, :func:`prewarm`, and :func:`decode_sjis`.
Parsing is pure-functional: construct a ``Document`` from source text
(or Shift_JIS / UTF-8 bytes via :meth:`Document.from_sjis`) and call the
render / inspection methods.

Inspection methods come in two flavours:

* the parsed accessors — :meth:`Document.diagnostics`, :meth:`Document.nodes`,
  :meth:`Document.pairs`, :meth:`Document.container_pairs` — return native
  ``list[dict]``;
* the ``*_json()`` accessors return the raw, byte-identical wire envelope
  string (``{"schema_version": 1, "data": [...]}``) shared with the
  WASM / FFI / Go drivers.

The compiled extension lives in the private submodule
``aozora_py._aozora_py``; import from ``aozora_py`` directly.
"""

from __future__ import annotations

import json
from importlib.metadata import PackageNotFoundError, version
from typing import Any

from . import _aozora_py
from ._aozora_py import decode_sjis, parse_to_html, prewarm, slugs_json

__all__ = [
    "Document",
    "decode_sjis",
    "parse_to_html",
    "prewarm",
    "slugs",
    "slugs_json",
    "__version__",
]

try:
    __version__ = version("aozora_py")
except PackageNotFoundError:  # source tree / editable build without dist metadata
    __version__ = "0.0.0+unknown"


def _envelope_data(envelope_json: str) -> list[dict[str, Any]]:
    """Parse a ``{"schema_version", "data"}`` wire string to its ``data`` list."""
    data: list[dict[str, Any]] = json.loads(envelope_json)["data"]
    return data


def slugs() -> list[dict[str, Any]]:
    """The canonical ``［＃…］`` slug catalogue as a list of dicts.

    Static — independent of any document. Each entry is
    ``{canonical, family, accepts_param, doc, partner}``. Use
    :func:`slugs_json` for the raw wire string.
    """
    return _envelope_data(slugs_json())


class Document:
    """A parsed Aozora Bunko document.

    ``unsendable``: a ``Document`` is pinned to the thread that created it
    (the parser owns a bump arena with interior mutability). Touching one
    from another thread raises ``RuntimeError`` rather than sharing
    unsoundly.
    """

    __slots__ = ("_native",)

    def __init__(self, source: str) -> None:
        self._native = _aozora_py.Document(source)

    @classmethod
    def _wrap(cls, native: _aozora_py.Document) -> Document:
        doc = cls.__new__(cls)
        doc._native = native
        return doc

    @classmethod
    def from_sjis(cls, data: bytes) -> Document:
        """Construct from raw bytes, auto-detecting Shift_JIS vs UTF-8.

        Real 青空文庫 archive files are Shift_JIS; pre-converted corpora
        are UTF-8. Both are accepted. Raises ``ValueError`` on bytes that
        are neither, or on a source over the 4 GiB span limit.
        """
        return cls._wrap(_aozora_py.Document.from_sjis(data))

    @property
    def source(self) -> str:
        """The source text this document was parsed from."""
        return self._native.source

    def source_byte_len(self) -> int:
        """Source length in UTF-8 bytes."""
        return self._native.source_byte_len()

    def to_html(self) -> str:
        """Render to semantic HTML5."""
        return self._native.to_html()

    def serialize(self) -> str:
        """Re-emit Aozora source text from the parse tree."""
        return self._native.serialize()

    # ── parsed accessors (native list[dict]) ──────────────────────────
    def diagnostics(self) -> list[dict[str, Any]]:
        """Diagnostics as a list of dicts (the parsed wire ``data`` array)."""
        return _envelope_data(self._native.diagnostics_json())

    def nodes(self) -> list[dict[str, Any]]:
        """Classified Aozora-node spans as a list of dicts."""
        return _envelope_data(self._native.nodes_json())

    def pairs(self) -> list[dict[str, Any]]:
        """Matched open/close pair links as a list of dicts."""
        return _envelope_data(self._native.pairs_json())

    def container_pairs(self) -> list[dict[str, Any]]:
        """Container open/close pairs (indent / warichu / …) as a list of dicts."""
        return _envelope_data(self._native.container_pairs_json())

    def gaiji_resolutions(self) -> list[dict[str, Any]]:
        """Resolved gaiji references (``※［＃…］``) as a list of dicts.

        Each entry is ``{span, description, mencode, codepoint, resolved}``;
        ``resolved`` is ``None`` when the reference can't be mapped to a glyph.
        """
        return _envelope_data(self._native.gaiji_resolutions_json())

    # ── raw wire accessors (byte-identical envelope strings) ──────────
    def diagnostics_json(self) -> str:
        """Raw diagnostics wire envelope (JSON string)."""
        return self._native.diagnostics_json()

    def nodes_json(self) -> str:
        """Raw nodes wire envelope (JSON string)."""
        return self._native.nodes_json()

    def pairs_json(self) -> str:
        """Raw pairs wire envelope (JSON string)."""
        return self._native.pairs_json()

    def container_pairs_json(self) -> str:
        """Raw container-pairs wire envelope (JSON string)."""
        return self._native.container_pairs_json()

    def gaiji_resolutions_json(self) -> str:
        """Raw gaiji-resolutions wire envelope (JSON string)."""
        return self._native.gaiji_resolutions_json()

    def __repr__(self) -> str:
        return f"<aozora_py.Document source_byte_len={self._native.source_byte_len()}>"
