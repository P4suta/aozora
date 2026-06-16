"""Core Document construction / render / serialize surface."""

import aozora_py


def test_construct_and_source():
    d = aozora_py.Document("hello")
    assert d.source == "hello"
    assert d.source_byte_len() == 5


def test_to_html_renders_ruby():
    d = aozora_py.Document("｜青梅《おうめ》")
    html = d.to_html()
    assert "ruby" in html


def test_serialize_round_trips():
    src = "｜青梅《おうめ》の街"
    assert aozora_py.Document(src).serialize() == src


def test_repr_mentions_document():
    assert "Document" in repr(aozora_py.Document("hello"))


def test_parse_to_html_function():
    assert "ruby" in aozora_py.parse_to_html("｜青梅《おうめ》")


def test_public_all():
    assert set(aozora_py.__all__) >= {
        "Document",
        "parse_to_html",
        "prewarm",
        "decode_sjis",
    }
