"""The ``version`` (build) and ``schema_version`` (wire) exports."""

import json

import aozora


def test_schema_version_matches_envelopes():
    """``schema_version()`` equals the ``schemaVersion`` every envelope stamps."""
    sv = aozora.schema_version()
    assert isinstance(sv, int)
    d = aozora.Document("plain text")
    assert json.loads(d.diagnostics_json())["schemaVersion"] == sv
    assert json.loads(aozora.slugs_json())["schemaVersion"] == sv


def test_version_is_a_nonempty_build_stamp():
    """``version()`` is the parser build stamp — a non-empty string."""
    v = aozora.version()
    assert isinstance(v, str)
    assert v, "build version string must not be empty"
    # Starts with the base semver triple (possibly followed by a
    # `-dev+g…` / `-nightly…` channel suffix).
    assert v[0].isdigit()


def test_build_version_and_package_version_are_distinct_concepts():
    """The build stamp and the dist-metadata package version both exist."""
    assert isinstance(aozora.version(), str)
    assert isinstance(aozora.__version__, str)
