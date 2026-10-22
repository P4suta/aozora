"""Checks parser build and wire schema versions."""

import json

import aozora


def test_schema_version_matches_envelopes():
    sv = aozora.schema_version()
    assert isinstance(sv, int)
    d = aozora.Document("plain text")
    assert json.loads(d.diagnostics_json())["schemaVersion"] == sv
    assert json.loads(aozora.slugs_json())["schemaVersion"] == sv


def test_version_is_a_nonempty_build_stamp():
    v = aozora.version()
    assert isinstance(v, str)
    assert v, "build version string must not be empty"
    assert v[0].isdigit()


def test_build_version_and_package_version_are_distinct_concepts():
    assert isinstance(aozora.version(), str)
    assert isinstance(aozora.__version__, str)
