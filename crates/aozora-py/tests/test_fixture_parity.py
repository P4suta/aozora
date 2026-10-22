"""Compares the Python extension with shared rendering fixtures.

JSON comparisons remove the optional storage newline from fixture files.
"""

from __future__ import annotations

from pathlib import Path

import pytest

import aozora

FIXTURES = Path(__file__).resolve().parents[2] / "aozora-conformance" / "fixtures" / "render"

SURFACES = [
    ("expected.html", lambda d: d.to_html()),
    ("expected.serialize.txt", lambda d: d.to_source()),
    ("expected.diagnostics.json", lambda d: d.diagnostics_json()),
    ("expected.nodes.json", lambda d: d.nodes_json()),
    ("expected.pairs.json", lambda d: d.pairs_json()),
    ("expected.container_pairs.json", lambda d: d.container_pairs_json()),
]


def _fixture_dirs() -> list[Path]:
    if not FIXTURES.is_dir():
        return []
    return sorted(p for p in FIXTURES.iterdir() if p.is_dir())


def test_fixture_root_is_present() -> None:
    assert FIXTURES.is_dir(), f"fixtures root missing: {FIXTURES}"
    assert (FIXTURES / "bouten").is_dir(), "expected the `bouten` fixture under the render group"
    assert _fixture_dirs(), "no render fixtures discovered"


@pytest.mark.parametrize("fixture", _fixture_dirs(), ids=lambda p: p.name)
def test_surface_parity(fixture: Path) -> None:
    source = (fixture / "source.txt").read_text(encoding="utf-8")
    doc = aozora.Document(source)
    for filename, accessor in SURFACES:
        golden = (fixture / filename).read_text(encoding="utf-8")
        if filename.endswith(".json"):
            golden = golden.removesuffix("\n")
        assert accessor(doc) == golden, f"{fixture.name}/{filename} drift"
