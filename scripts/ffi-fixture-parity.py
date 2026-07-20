#!/usr/bin/env python3

import ctypes
import pathlib
import sys


class AozoraBytes(ctypes.Structure):
    _fields_ = [
        ("ptr", ctypes.POINTER(ctypes.c_uint8)),
        ("len", ctypes.c_size_t),
        ("cap", ctypes.c_size_t),
    ]


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: ffi-fixture-parity.py <library> <fixtures>")
    library = ctypes.CDLL(str(pathlib.Path(sys.argv[1]).resolve()))
    fixtures = pathlib.Path(sys.argv[2])

    library.aozora_document_new.argtypes = [
        ctypes.POINTER(ctypes.c_uint8),
        ctypes.c_size_t,
        ctypes.POINTER(ctypes.c_void_p),
    ]
    library.aozora_document_new.restype = ctypes.c_int
    library.aozora_document_free.argtypes = [ctypes.c_void_p]
    library.aozora_bytes_free.argtypes = [AozoraBytes]

    surfaces = {
        "aozora_document_to_html": "expected.html",
        "aozora_document_to_source": "expected.serialize.txt",
        "aozora_document_diagnostics_json": "expected.diagnostics.json",
        "aozora_document_nodes_json": "expected.nodes.json",
        "aozora_document_pairs_json": "expected.pairs.json",
        "aozora_document_container_pairs_json": "expected.container_pairs.json",
    }
    for symbol in surfaces:
        accessor = getattr(library, symbol)
        accessor.argtypes = [ctypes.c_void_p, ctypes.POINTER(AozoraBytes)]
        accessor.restype = ctypes.c_int

    directories = sorted(path for path in fixtures.iterdir() if path.is_dir())
    if not directories:
        raise AssertionError(f"no fixtures under {fixtures}")
    checks = 0
    for directory in directories:
        source = (directory / "source.txt").read_bytes()
        source_buffer = (ctypes.c_uint8 * len(source)).from_buffer_copy(source)
        document = ctypes.c_void_p()
        status = library.aozora_document_new(
            source_buffer, len(source), ctypes.byref(document)
        )
        if status != 0 or not document.value:
            raise AssertionError(f"{directory.name}: document_new returned {status}")
        try:
            for symbol, filename in surfaces.items():
                output = AozoraBytes()
                status = getattr(library, symbol)(document, ctypes.byref(output))
                if status != 0:
                    raise AssertionError(
                        f"{directory.name}/{symbol} returned {status}"
                    )
                try:
                    actual = ctypes.string_at(output.ptr, output.len)
                finally:
                    library.aozora_bytes_free(output)
                expected = (directory / filename).read_bytes()
                if filename.endswith(".json"):
                    expected = expected.removesuffix(b"\n")
                if actual != expected:
                    raise AssertionError(f"{directory.name}/{filename} drift")
                checks += 1
        finally:
            library.aozora_document_free(document)
    print(f"ffi-fixture-parity: {checks} checks passed")


if __name__ == "__main__":
    main()
