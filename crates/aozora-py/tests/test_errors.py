"""Checks decoding errors through ``ValueError``."""

import pytest

import aozora

INVALID_BYTES = b"\x81\xff"


def test_decode_sjis_rejects_malformed():
    with pytest.raises(ValueError):
        aozora.decode_sjis(INVALID_BYTES)


def test_from_sjis_rejects_malformed():
    with pytest.raises(ValueError):
        aozora.Document.from_bytes(INVALID_BYTES)
