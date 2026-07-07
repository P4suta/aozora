"""Slug catalogue + full-document gaiji resolution (parity with the WASM driver)."""

import json

import aozora


def _envelope(s: str) -> list:
    obj = json.loads(s)
    assert obj["schemaVersion"] == 2
    assert isinstance(obj["data"], list)
    return obj["data"]


def test_slugs_catalogue_nonempty_and_shaped():
    data = _envelope(aozora.slugs_json())
    assert data, "slug catalogue should not be empty"
    for entry in data:
        assert {"canonical", "family", "accepts_param", "doc", "partner"} <= set(entry)
    # No shipped slug should degrade to the catch-all "unknown" family.
    assert all(e["family"] != "unknown" for e in data)


def test_slugs_parsed_matches_json():
    assert aozora.slugs() == _envelope(aozora.slugs_json())


def test_gaiji_resolutions_empty_for_plain_text():
    d = aozora.Document("plain text")
    assert d.gaiji() == []
    assert d.gaiji_json() == '{"schemaVersion":2,"data":[]}'


def test_gaiji_resolutions_resolves_reference():
    d = aozora.Document("前※［＃「々」］後")
    res = d.gaiji()
    assert len(res) == 1
    g = res[0]
    assert g["description"] == "々"
    assert g["resolved"] == "々"
    assert "start" in g["span"] and "end" in g["span"]


def test_gaiji_parsed_matches_json():
    d = aozora.Document("※［＃「々」］")
    assert d.gaiji() == _envelope(d.gaiji_json())
