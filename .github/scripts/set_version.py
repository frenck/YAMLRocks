#!/usr/bin/env python3
"""Set the package version in Cargo.toml, pyproject.toml, and Cargo.lock from a
release tag.

Usage: ``set_version.py <tag>`` where ``<tag>`` is e.g. ``v1.2.3`` or ``1.2.3``.
The leading ``v`` is stripped and the version is lowercased. The two manifests
have their first ``version = "..."`` line rewritten in place, and the crate's own
entry in Cargo.lock is updated to match. That last step matters because the wheel
builds run ``maturin ... --locked``: leaving Cargo.lock at the old version makes
cargo refuse the bumped manifest ("cannot update the lock file ... --locked").
"""

from __future__ import annotations

import pathlib
import re
import sys


def set_version(path: pathlib.Path, version: str) -> None:
    text = path.read_text(encoding="utf-8")
    new_text, count = re.subn(
        r'(?m)^version = ".*"$', f'version = "{version}"', text, count=1
    )
    if count != 1:
        raise SystemExit(f"could not find a version line in {path}")
    path.write_text(new_text, encoding="utf-8")
    print(f"Set {path} version to {version}")


def package_name(cargo_toml: pathlib.Path) -> str:
    """The crate name from the ``[package]`` table of Cargo.toml."""
    in_package = False
    for line in cargo_toml.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_package = stripped == "[package]"
        elif in_package:
            match = re.match(r'name = "(.+?)"', stripped)
            if match:
                return match.group(1)
    raise SystemExit(f"could not find a [package] name in {cargo_toml}")


def set_lock_version(path: pathlib.Path, name: str, version: str) -> None:
    """Update the crate's own ``[[package]]`` entry in Cargo.lock. In the lock
    format the ``version`` line immediately follows the ``name`` line."""
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    for i, line in enumerate(lines):
        if line.strip() == f'name = "{name}"':
            after = lines[i + 1] if i + 1 < len(lines) else ""
            if not after.startswith("version = "):
                raise SystemExit(f"unexpected layout for {name} package in {path}")
            lines[i + 1] = f'version = "{version}"\n'
            path.write_text("".join(lines), encoding="utf-8")
            print(f"Set {path} {name} version to {version}")
            return
    raise SystemExit(f"could not find the {name} package in {path}")


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: set_version.py <tag>")
    version = sys.argv[1].removeprefix("v").lower()
    if not re.fullmatch(r"[0-9][0-9a-z.+-]*", version):
        raise SystemExit(f"refusing to set suspicious version: {version!r}")
    cargo_toml = pathlib.Path("Cargo.toml")
    set_version(cargo_toml, version)
    set_version(pathlib.Path("pyproject.toml"), version)
    set_lock_version(pathlib.Path("Cargo.lock"), package_name(cargo_toml), version)


if __name__ == "__main__":
    main()
