"""Slug catalogue + full-document gaiji resolution (parity with the WASM driver)."""

import json

import aozora_py


def _envelope(s: str) -> list:
    obj = json.loads(s)
    assert obj["schema_version"] == 1
    assert isinstance(obj["data"], list)
    return obj["data"]


def test_slugs_catalogue_nonempty_and_shaped():
    data = _envelope(aozora_py.slugs_json())
    assert data, "slug catalogue should not be empty"
    for entry in data:
        assert {"canonical", "family", "accepts_param", "doc", "partner"} <= set(entry)
    # No shipped slug should degrade to the catch-all "unknown" family.
    assert all(e["family"] != "unknown" for e in data)


def test_slugs_parsed_matches_json():
    assert aozora_py.slugs() == _envelope(aozora_py.slugs_json())


def test_gaiji_resolutions_empty_for_plain_text():
    d = aozora_py.Document("plain text")
    assert d.gaiji_resolutions() == []
    assert d.gaiji_resolutions_json() == '{"schema_version":1,"data":[]}'


def test_gaiji_resolutions_resolves_reference():
    d = aozora_py.Document("前※［＃「々」］後")
    res = d.gaiji_resolutions()
    assert len(res) == 1
    g = res[0]
    assert g["description"] == "々"
    assert g["resolved"] == "々"
    assert "start" in g["span"] and "end" in g["span"]


def test_gaiji_parsed_matches_json():
    d = aozora_py.Document("※［＃「々」］")
    assert d.gaiji_resolutions() == _envelope(d.gaiji_resolutions_json())
