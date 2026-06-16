"""The parser arena is thread-pinned (`#[pyclass(unsendable)]`).

A ``Document`` created on one thread must not be usable from another. PyO3
enforces this affinity at the boundary: cross-thread access trips a Rust
panic, surfaced to Python as ``pyo3_runtime.PanicException`` whose message
names the offending class as *unsendable* — rather than silently sharing
the bump arena unsoundly.
"""

import threading

import aozora_py


def test_cross_thread_access_is_rejected():
    doc = aozora_py.Document("｜青梅《おうめ》")  # created on the main thread
    captured: list[BaseException] = []

    def worker() -> None:
        try:
            doc.to_html()  # touch the native object from another thread
        except BaseException as exc:  # noqa: BLE001 - capturing for assertion
            captured.append(exc)

    t = threading.Thread(target=worker)
    t.start()
    t.join()

    assert len(captured) == 1, "cross-thread access should have raised"
    # PyO3 0.29 surfaces the affinity violation as PanicException.
    assert type(captured[0]).__name__ == "PanicException"
    assert "unsendable" in str(captured[0])


def test_same_thread_access_is_fine():
    doc = aozora_py.Document("｜青梅《おうめ》")
    assert "ruby" in doc.to_html()  # no error on the creating thread
