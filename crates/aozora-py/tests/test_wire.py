"""Raw ``*_json()`` accessors: envelope shape + cross-driver parity."""

import json

import aozora

EMPTY_ENVELOPE = '{"schemaVersion":2,"data":[]}'


def _envelope(s: str) -> list:
    obj = json.loads(s)
    assert obj["schemaVersion"] == 2
    assert isinstance(obj["data"], list)
    return obj["data"]


def test_clean_input_is_empty_envelope_for_every_endpoint():
    d = aozora.Document("plain text")
    assert d.diagnostics_json() == EMPTY_ENVELOPE
    assert d.nodes_json() == EMPTY_ENVELOPE
    assert d.pairs_json() == EMPTY_ENVELOPE
    assert d.container_pairs_json() == EMPTY_ENVELOPE


def test_pua_collision_surfaces_in_diagnostics():
    d = aozora.Document("abcdef")
    data = _envelope(d.diagnostics_json())
    assert any(x["kind"] == "source_contains_pua" for x in data)


def test_ruby_classified_in_nodes():
    d = aozora.Document("｜青梅《おうめ》")
    data = _envelope(d.nodes_json())
    assert any(x["kind"] == "ruby" for x in data)


def test_ruby_pair_emitted():
    d = aozora.Document("｜青梅《おうめ》")
    data = _envelope(d.pairs_json())
    assert any(x["kind"] == "ruby" for x in data)
    assert all("open" in x and "close" in x for x in data)


def test_nodes_are_in_source_order():
    d = aozora.Document("｜山《やま》。｜川《かわ》。｜空《そら》。")
    data = _envelope(d.nodes_json())
    starts = [x["span"]["start"] for x in data]
    assert starts == sorted(starts)
