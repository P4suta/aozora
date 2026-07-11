"""Cross-surface parity gate — Python (PyO3) channel.

One golden authority (``crates/aozora-conformance/fixtures/render``), N
thin walkers. This walker drives the compiled ``aozora`` extension over
every render fixture and asserts each accessor is byte-identical to the
golden the in-process ``render_gate`` pins. A binding that reframes,
re-orders, or drops a byte lights up here without duplicating the golden
per channel.

All six surfaces are byte-exact: ``to_html`` / ``to_source`` and the four
``*_json()`` accessors each return the raw shared ``aozora::json`` bytes
with no framing (unlike the line-oriented ``aozora inspect`` CLI, which
appends a trailing newline).
"""

from __future__ import annotations

from pathlib import Path

import pytest

import aozora

# Resolve the shared golden corpus relative to THIS file so the walk is
# independent of pytest's working directory (repo root under both
# `just smoke-py` and the ci.yml `python-wheel` job).
#   parents[0] = tests/  parents[1] = aozora-py/  parents[2] = crates/
FIXTURES = Path(__file__).resolve().parents[2] / "aozora-conformance" / "fixtures" / "render"

# surface file -> Document accessor producing the byte-identical output.
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
    """Guard against a silent no-op: if path resolution breaks, this fails
    loudly instead of the parametrized walk collecting zero cases."""
    assert FIXTURES.is_dir(), f"fixtures root missing: {FIXTURES}"
    assert (FIXTURES / "bouten").is_dir(), "expected the `bouten` fixture under the render group"
    assert _fixture_dirs(), "no render fixtures discovered"


@pytest.mark.parametrize("fixture", _fixture_dirs(), ids=lambda p: p.name)
def test_surface_parity(fixture: Path) -> None:
    source = (fixture / "source.txt").read_text(encoding="utf-8")
    doc = aozora.Document(source)
    for filename, accessor in SURFACES:
        golden = (fixture / filename).read_text(encoding="utf-8")
        assert accessor(doc) == golden, f"{fixture.name}/{filename} drift"
