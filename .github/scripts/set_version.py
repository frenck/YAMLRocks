#!/usr/bin/env python3
"""Set the package version in Cargo.toml and pyproject.toml from a release tag.

Usage: ``set_version.py <tag>`` where ``<tag>`` is e.g. ``v1.2.3`` or ``1.2.3``.
The leading ``v`` is stripped and the version is lowercased. Both manifests have
their first ``version = "..."`` line rewritten in place.
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


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: set_version.py <tag>")
    version = sys.argv[1].removeprefix("v").lower()
    if not re.fullmatch(r"[0-9][0-9a-z.+-]*", version):
        raise SystemExit(f"refusing to set suspicious version: {version!r}")
    set_version(pathlib.Path("Cargo.toml"), version)
    set_version(pathlib.Path("pyproject.toml"), version)


if __name__ == "__main__":
    main()
