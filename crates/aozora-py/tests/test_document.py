"""Core Document construction / render / to_source surface."""

import aozora


def test_construct_and_source():
    d = aozora.Document("hello")
    assert d.source == "hello"
    assert d.source_byte_len() == 5


def test_to_html_renders_ruby():
    d = aozora.Document("｜青梅《おうめ》")
    html = d.to_html()
    assert "ruby" in html


def test_serialize_round_trips():
    src = "｜青梅《おうめ》の街"
    assert aozora.Document(src).to_source() == src


def test_repr_mentions_document():
    assert "Document" in repr(aozora.Document("hello"))


def test_parse_to_html_function():
    assert "ruby" in aozora.parse_to_html("｜青梅《おうめ》")


def test_public_all():
    assert set(aozora.__all__) >= {
        "Document",
        "parse_to_html",
        "prewarm",
        "decode_sjis",
    }
