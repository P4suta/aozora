"""Shift_JIS decoding — the path real 青空文庫 archive files take."""

import aozora


def test_decode_sjis_round_trip():
    text = "青空文庫"
    assert aozora.decode_sjis(text.encode("shift_jis")) == text


def test_from_sjis_round_trip():
    text = "吾輩は猫である"
    doc = aozora.Document.from_bytes(text.encode("shift_jis"))
    assert doc.source == text


def test_from_sjis_also_accepts_utf8():
    # decode_auto sniffs UTF-8 first, so already-decoded mirrors work too.
    text = "青空"
    doc = aozora.Document.from_bytes(text.encode("utf-8"))
    assert doc.source == text
