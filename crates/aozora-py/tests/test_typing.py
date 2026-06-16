"""Packaging of the typing surface (PEP 561) + stub-vs-runtime drift guard."""

import ast
import pathlib

import aozora_py
from aozora_py import _aozora_py

_PKG_DIR = pathlib.Path(aozora_py.__file__).parent


def test_py_typed_marker_is_packaged():
    assert (_PKG_DIR / "py.typed").is_file()


def test_native_stub_is_packaged():
    assert (_PKG_DIR / "_aozora_py.pyi").is_file()


def _stub_tree() -> ast.Module:
    return ast.parse((_PKG_DIR / "_aozora_py.pyi").read_text(encoding="utf-8"))


def test_stub_top_level_symbols_exist_at_runtime():
    tree = _stub_tree()
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)):
            assert hasattr(_aozora_py, node.name), (
                f"{node.name} declared in _aozora_py.pyi but missing at runtime"
            )


def test_stub_document_methods_exist_at_runtime():
    tree = _stub_tree()
    doc_cls = next(
        n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "Document"
    )
    for member in doc_cls.body:
        if isinstance(member, ast.FunctionDef) and member.name != "__init__":
            assert hasattr(_aozora_py.Document, member.name), (
                f"Document.{member.name} declared in stub but missing at runtime"
            )
