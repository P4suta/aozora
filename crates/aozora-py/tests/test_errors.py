"""Fallible entry points surface ``ValueError`` (mapped from DecodeError)."""

import pytest

import aozora

# 0x81 is a Shift_JIS lead byte; 0xFF is not a valid trail → malformed.
# Also invalid UTF-8, so decode_auto (UTF-8-first) falls through to the
# strict Shift_JIS path and errors there too.
INVALID_BYTES = b"\x81\xff"


def test_decode_sjis_rejects_malformed():
    with pytest.raises(ValueError):
        aozora.decode_sjis(INVALID_BYTES)


def test_from_sjis_rejects_malformed():
    with pytest.raises(ValueError):
        aozora.Document.from_bytes(INVALID_BYTES)
