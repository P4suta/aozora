"""Aozora Bunko notation parser — Python bindings.

The public surface is the :class:`Document` class plus the module-level
helpers :func:`parse_to_html`, :func:`prewarm`, :func:`decode_sjis`,
:func:`slugs`, :func:`version`, and :func:`schema_version`.
Parsing is pure-functional: construct a ``Document`` from source text
(or Shift_JIS / UTF-8 bytes via :meth:`Document.from_bytes`) and call the
render / inspection methods.

Two version numbers, deliberately distinct: :func:`version` is the
parser engine's channel-aware *build* stamp (shared with the WASM /
Extism / Go drivers), while ``__version__`` is the installed PyPI
*package* version from distribution metadata. :func:`schema_version`
is the cross-driver *wire* schema version stamped into every
``*_json()`` envelope.

Inspection methods come in two flavours:

* the parsed accessors return generated, typed Python dataclasses;
* the ``*_json()`` accessors return the raw wire envelope string used by
  CLI, FFI, Extism, and Go.

The compiled extension lives in the private submodule
``aozora._aozora``; import from ``aozora`` directly.
"""

from __future__ import annotations

import json
from importlib.metadata import PackageNotFoundError
from importlib.metadata import version as _dist_version

from . import _aozora
from ._aozora import (
    decode_sjis,
    parse_to_html,
    prewarm,
    schema_version,
    slugs_json,
    version,
)
from .wire_types import (
    ContainerPair,
    Diagnostic,
    GaijiResolution,
    Node,
    Pair,
    Slug,
    Span,
)
from .wire_types import (
    AozoraContainerPairsEnvelope as _ContainerPairsEnvelope,
)
from .wire_types import AozoraDiagnosticsEnvelope as _DiagnosticsEnvelope
from .wire_types import AozoraGaijiEnvelope as _GaijiEnvelope
from .wire_types import AozoraNodesEnvelope as _NodesEnvelope
from .wire_types import AozoraPairsEnvelope as _PairsEnvelope
from .wire_types import AozoraSlugsEnvelope as _SlugsEnvelope

__all__ = [
    "Document",
    "ContainerPair",
    "Diagnostic",
    "GaijiResolution",
    "Node",
    "Pair",
    "Slug",
    "Span",
    "decode_sjis",
    "parse_to_html",
    "prewarm",
    "schema_version",
    "slugs",
    "slugs_json",
    "version",
    "__version__",
]

try:
    __version__ = _dist_version("aozora")
except PackageNotFoundError:  # source tree / editable build without dist metadata
    __version__ = "0.0.0+unknown"


def slugs() -> list[Slug]:
    """The canonical ``［＃…］`` completion catalogue."""
    return _SlugsEnvelope.from_dict(json.loads(slugs_json())).data


class Document:
    """A parsed Aozora Bunko document.

    ``unsendable``: a ``Document`` is pinned to the thread that created it
    (a conservative marker; the underlying document is thread-safe but the
    handle is pinned to its constructing thread). Touching one
    from another thread raises ``RuntimeError`` rather than sharing
    unsoundly.
    """

    __slots__ = ("_native",)

    def __init__(self, source: str) -> None:
        self._native = _aozora.Document(source)

    @classmethod
    def _wrap(cls, native: _aozora.Document) -> Document:
        doc = cls.__new__(cls)
        doc._native = native
        return doc

    @classmethod
    def from_bytes(cls, data: bytes) -> Document:
        """Construct from raw bytes, auto-detecting Shift_JIS vs UTF-8.

        Real 青空文庫 archive files are Shift_JIS; pre-converted corpora
        are UTF-8. Both are accepted. Raises ``ValueError`` on bytes that
        are neither, or on a source over the 4 GiB span limit.
        """
        return cls._wrap(_aozora.Document.from_bytes(data))

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

    def to_source(self) -> str:
        """Re-emit Aozora source text from the parse tree."""
        return self._native.to_source()

    def diagnostics_text(self) -> str:
        """Diagnostics as a plain-text report (``miette``-free).

        One block per diagnostic with its code, span, message, and the
        offending source slice; a clean parse returns the empty string.
        Use :meth:`diagnostics` for the structured list or
        :meth:`diagnostics_json` for the raw wire envelope.
        """
        return self._native.diagnostics_text()

    # ── parsed accessors ──────────────────────────────────────────────
    def diagnostics(self) -> list[Diagnostic]:
        """Diagnostics as typed native values."""
        return _DiagnosticsEnvelope.from_dict(
            json.loads(self._native.diagnostics_json())
        ).data

    def nodes(self) -> list[Node]:
        """Classified Aozora-node spans as typed native values."""
        return _NodesEnvelope.from_dict(json.loads(self._native.nodes_json())).data

    def pairs(self) -> list[Pair]:
        """Matched open/close pair links as typed native values."""
        return _PairsEnvelope.from_dict(json.loads(self._native.pairs_json())).data

    def container_pairs(self) -> list[ContainerPair]:
        """Container open/close pairs as typed native values."""
        return _ContainerPairsEnvelope.from_dict(
            json.loads(self._native.container_pairs_json())
        ).data

    def gaiji(self) -> list[GaijiResolution]:
        """Resolved gaiji references as typed native values."""
        return _GaijiEnvelope.from_dict(json.loads(self._native.gaiji_json())).data

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

    def gaiji_json(self) -> str:
        """Raw gaiji-resolutions wire envelope (JSON string)."""
        return self._native.gaiji_json()

    def __repr__(self) -> str:
        return f"<aozora.Document source_byte_len={self._native.source_byte_len()}>"
