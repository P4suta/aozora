"""Parsed accessors return native ``list[dict]`` equal to the wire ``data``."""

import json

import aozora


def test_parsed_accessors_match_json_data():
    d = aozora.Document("｜青梅《おうめ》や［＃改ページ］\n≪秘密≫")
    pairs = [
        (d.diagnostics(), d.diagnostics_json()),
        (d.nodes(), d.nodes_json()),
        (d.pairs(), d.pairs_json()),
        (d.container_pairs(), d.container_pairs_json()),
    ]
    for parsed, raw in pairs:
        assert isinstance(parsed, list)
        assert parsed == json.loads(raw)["data"]


def test_parsed_entries_are_dicts_with_kind_and_span():
    d = aozora.Document("｜青梅《おうめ》")
    nodes = d.nodes()
    assert nodes, "ruby source should classify at least one node"
    for entry in nodes:
        assert isinstance(entry, dict)
        assert "kind" in entry
        assert "span" in entry


def test_clean_input_returns_empty_lists():
    d = aozora.Document("plain text")
    assert d.diagnostics() == []
    assert d.nodes() == []
    assert d.pairs() == []
    assert d.container_pairs() == []
