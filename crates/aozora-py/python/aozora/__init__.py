"""Parses and inspects Aozora documents through :class:`Document`.

Parsed accessors return typed Python values, while ``*_json()`` accessors return wire envelopes.
Use :func:`version` for the engine build, ``__version__`` for the installed package, and :func:`schema_version` for wire compatibility.
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
except PackageNotFoundError:
    __version__ = "0.0.0+unknown"


def slugs() -> list[Slug]:
    """The canonical ``［＃…］`` completion catalogue."""
    return _SlugsEnvelope.from_dict(json.loads(slugs_json())).data


class Document:
    """A parsed Aozora Bunko document.

    Access from a thread other than the creating thread raises ``pyo3_runtime.PanicException``.
    Construction raises ``ValueError`` if the source exceeds ``2**32 - 1`` UTF-8 bytes.
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
        """Creates a document from bytes, preferring UTF-8 and otherwise decoding ``Shift_JIS``.

        Raises ``ValueError`` for invalid encoding or decoded source exceeding ``2**32 - 1`` UTF-8 bytes.
        """
        return cls._wrap(_aozora.Document.from_bytes(data))

    @property
    def source(self) -> str:
        """Returns the document's source text."""
        return self._native.source

    def source_byte_len(self) -> int:
        """Source length in UTF-8 bytes."""
        return self._native.source_byte_len()

    def to_html(self) -> str:
        """Renders the document as HTML."""
        return self._native.to_html()

    def to_source(self) -> str:
        """Re-emit Aozora source text from the parse tree."""
        return self._native.to_source()

    def diagnostics_text(self) -> str:
        """Returns a plain-text diagnostic report, or an empty string for a clean parse.
        """
        return self._native.diagnostics_text()

    def diagnostics(self) -> list[Diagnostic]:
        """Diagnostics as typed native values."""
        return _DiagnosticsEnvelope.from_dict(
            json.loads(self._native.diagnostics_json())
        ).data

    def nodes(self) -> list[Node]:
        """Returns typed nodes with original-source UTF-8 byte spans."""
        return _NodesEnvelope.from_dict(json.loads(self._native.nodes_json())).data

    def pairs(self) -> list[Pair]:
        """Returns typed delimiter pairs with original-source UTF-8 byte spans."""
        return _PairsEnvelope.from_dict(json.loads(self._native.pairs_json())).data

    def container_pairs(self) -> list[ContainerPair]:
        """Returns typed container pairs with original-source UTF-8 byte spans."""
        return _ContainerPairsEnvelope.from_dict(
            json.loads(self._native.container_pairs_json())
        ).data

    def gaiji(self) -> list[GaijiResolution]:
        """Resolved gaiji references as typed native values."""
        return _GaijiEnvelope.from_dict(json.loads(self._native.gaiji_json())).data

    def diagnostics_json(self) -> str:
        """Returns the diagnostics wire envelope as a JSON string."""
        return self._native.diagnostics_json()

    def nodes_json(self) -> str:
        """Returns the nodes wire envelope as a JSON string."""
        return self._native.nodes_json()

    def pairs_json(self) -> str:
        """Returns the pairs wire envelope as a JSON string."""
        return self._native.pairs_json()

    def container_pairs_json(self) -> str:
        """Returns the container pairs wire envelope as a JSON string."""
        return self._native.container_pairs_json()

    def gaiji_json(self) -> str:
        """Returns the gaiji wire envelope as a JSON string."""
        return self._native.gaiji_json()

    def __repr__(self) -> str:
        return f"<aozora.Document source_byte_len={self._native.source_byte_len()}>"
