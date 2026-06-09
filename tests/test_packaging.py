"""Packaging guarantees that ship inside the wheel.

These guard distribution-level contracts that unit tests would not catch, since
they depend on what maturin actually bundles rather than on runtime behavior.
"""

from __future__ import annotations

import pathlib

import yamlrocks


def _package_dir() -> pathlib.Path:
    assert yamlrocks.__file__ is not None
    return pathlib.Path(yamlrocks.__file__).parent


def test_ships_py_typed_marker():
    """The PEP 561 ``py.typed`` marker must ship beside the package.

    Without it, type checkers ignore the bundled stubs and treat ``yamlrocks``
    as untyped (``Any``), emitting ``import-untyped`` for downstream users. The
    marker is what makes the type information take effect.
    """
    assert (_package_dir() / "py.typed").is_file()


def test_ships_type_stub():
    """The inline type stub for the native module ships alongside it."""
    assert (_package_dir() / "__init__.pyi").is_file()
