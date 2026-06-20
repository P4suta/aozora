"""Packaging of the typing surface (PEP 561) + stub-vs-runtime drift guard."""

import ast
import pathlib

import aozora
from aozora import _aozora

_PKG_DIR = pathlib.Path(aozora.__file__).parent


def test_py_typed_marker_is_packaged():
    assert (_PKG_DIR / "py.typed").is_file()


def test_native_stub_is_packaged():
    assert (_PKG_DIR / "_aozora.pyi").is_file()


def _stub_tree() -> ast.Module:
    return ast.parse((_PKG_DIR / "_aozora.pyi").read_text(encoding="utf-8"))


def test_stub_top_level_symbols_exist_at_runtime():
    tree = _stub_tree()
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)):
            assert hasattr(_aozora, node.name), (
                f"{node.name} declared in _aozora.pyi but missing at runtime"
            )


def test_stub_document_methods_exist_at_runtime():
    tree = _stub_tree()
    doc_cls = next(
        n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "Document"
    )
    for member in doc_cls.body:
        if isinstance(member, ast.FunctionDef) and member.name != "__init__":
            assert hasattr(_aozora.Document, member.name), (
                f"Document.{member.name} declared in stub but missing at runtime"
            )
