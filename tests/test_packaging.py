"""Packaging guarantees that ship inside the wheel.

These guard distribution-level contracts that unit tests would not catch, since
they depend on what maturin actually bundles rather than on runtime behavior.
"""

from __future__ import annotations

import pathlib
import subprocess
import sys

import yamlrocks

_SET_VERSION = (
    pathlib.Path(__file__).resolve().parents[1]
    / ".github"
    / "scripts"
    / "set_version.py"
)


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


def test_set_version_bumps_cargo_lock_in_lockstep(tmp_path):
    """``set_version.py`` must update the crate's own entry in Cargo.lock, not
    just the manifests. The wheel builds run ``maturin ... --locked``, so a stale
    lock makes cargo reject the bumped manifest ("cannot update the lock file ...
    --locked") and the whole release fails. Run the real script against fixture
    files and assert every version moves together, leaving dependency pins alone.
    """
    (tmp_path / "Cargo.toml").write_text(
        '[package]\nname = "yamlrocks"\nversion = "0.1.0"\n\n'
        '[lib]\nname = "_yamlrocks"\n',
        encoding="utf-8",
    )
    (tmp_path / "pyproject.toml").write_text(
        '[project]\nname = "yamlrocks"\nversion = "0.1.0"\n', encoding="utf-8"
    )
    # A realistic lock: our own crate plus a dependency that must not be touched.
    (tmp_path / "Cargo.lock").write_text(
        "version = 4\n\n"
        '[[package]]\nname = "memchr"\nversion = "2.7.4"\n\n'
        '[[package]]\nname = "yamlrocks"\nversion = "0.1.0"\n'
        'dependencies = [\n "memchr",\n]\n',
        encoding="utf-8",
    )

    subprocess.run(
        [sys.executable, str(_SET_VERSION), "v2.3.4"], cwd=tmp_path, check=True
    )

    assert 'version = "2.3.4"' in (tmp_path / "Cargo.toml").read_text("utf-8")
    assert 'version = "2.3.4"' in (tmp_path / "pyproject.toml").read_text("utf-8")
    lock = (tmp_path / "Cargo.lock").read_text("utf-8")
    assert 'name = "yamlrocks"\nversion = "2.3.4"' in lock
    # The dependency pin is left exactly as it was.
    assert 'name = "memchr"\nversion = "2.7.4"' in lock
