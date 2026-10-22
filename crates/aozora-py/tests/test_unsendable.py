"""Checks access on the creating thread and rejection from other threads."""

import threading

import aozora


def test_cross_thread_access_is_rejected():
    doc = aozora.Document("｜青梅《おうめ》")
    captured: list[BaseException] = []

    def worker() -> None:
        try:
            doc.to_html()
        except BaseException as exc:  # noqa: BLE001 - capturing for assertion
            captured.append(exc)

    t = threading.Thread(target=worker)
    t.start()
    t.join()

    assert len(captured) == 1, "cross-thread access should have raised"
    assert type(captured[0]).__name__ == "PanicException"
    assert "unsendable" in str(captured[0])


def test_same_thread_access_is_fine():
    doc = aozora.Document("｜青梅《おうめ》")
    assert "ruby" in doc.to_html()
