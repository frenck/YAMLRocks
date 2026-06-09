"""Continuous performance benchmarks for YAMLRocks, run by CodSpeed.

Unlike ``bench/bench.py`` (which compares YAMLRocks against other libraries for a
one-off report), this module measures YAMLRocks's *own* operations so CodSpeed can
track them across commits and flag regressions on every pull request.

Each test wraps the measured call in the ``benchmark`` fixture from
``pytest-codspeed``; only the code inside that call is measured, so payload
construction and include-tree setup stay out of the timing. These live under
``bench/`` (outside ``testpaths``), so a normal ``pytest`` run does not collect
them; CI runs them explicitly with ``pytest bench --codspeed``.
"""

from __future__ import annotations

import pytest

import yamlrocks

# -- Payloads (representative real-world shapes) -----------------------------

SMALL = b"""\
name: my-app
version: 1.2.3
debug: false
port: 8080
tags:
  - web
  - api
owner:
  name: Alice
  email: alice@example.com
"""

MEDIUM = (
    b"apiVersion: apps/v1\n"
    b"kind: Deployment\n"
    b"metadata:\n  name: nginx\n  labels:\n    app: nginx\n"
    b"spec:\n  replicas: 3\n  template:\n    spec:\n      containers:\n"
    + b"".join(
        b"        - name: c%d\n          image: nginx:1.25\n          ports:\n"
        b"            - containerPort: %d\n" % (i, 8000 + i)
        for i in range(10)
    )
)

# A large mapping with many repeated keys (the common config shape).
LARGE = b"items:\n" + b"".join(
    b"  - id: %d\n    name: item-%d\n    enabled: true\n    score: %d.5\n" % (i, i, i)
    for i in range(500)
)

# A deeply nested document.
DEEP = (
    b"".join(b"  " * i + b"level%d:\n" % i for i in range(30))
    + b"  " * 30
    + b"value: 1\n"
)

# Anchor/alias-heavy: one shared block referenced many times (the shared-config
# shape). Exercises anchor registration and alias expansion, guarding the
# memoized per-anchor node-count against regression.
ALIAS = (
    b"shared: &shared\n  retries: 3\n  timeout: 30\n  tags: [a, b, c]\n"
    b"refs:\n" + b"".join(b"  ref%d: *shared\n" % i for i in range(300))
)

# Tag-heavy: every value carries an explicit core-schema tag, exercising the
# resolver's tagged-scalar path.
TAGGED = b"items:\n" + b"".join(
    b"  - name: !!str item%d\n    count: !!int %d\n    ratio: !!float %d.5\n"
    % (i, i, i)
    for i in range(300)
)

PAYLOADS = {
    "small": SMALL,
    "medium": MEDIUM,
    "large": LARGE,
    "deep": DEEP,
    "alias": ALIAS,
    "tagged": TAGGED,
}


# -- Parse (loads) -----------------------------------------------------------


@pytest.mark.parametrize("name", list(PAYLOADS))
def test_loads(benchmark, name):
    """Parse each representative payload into native Python objects."""
    data = PAYLOADS[name]
    benchmark(yamlrocks.loads, data)


# -- Serialize (dumps) -------------------------------------------------------


@pytest.mark.parametrize("name", list(PAYLOADS))
def test_dumps(benchmark, name):
    """Serialize the parsed object back to YAML bytes."""
    obj = yamlrocks.loads(PAYLOADS[name])
    benchmark(yamlrocks.dumps, obj)


# -- Round-trip --------------------------------------------------------------


def test_roundtrip_load(benchmark):
    """Parse into a structure-preserving ``YAMLRocksDocument``."""
    benchmark(yamlrocks.loads, LARGE, option=yamlrocks.OPT_ROUND_TRIP)


def test_roundtrip_emit(benchmark):
    """Re-emit an unmodified round-trip document (the byte-for-byte path)."""
    doc = yamlrocks.loads(LARGE, option=yamlrocks.OPT_ROUND_TRIP)
    # OPT_ROUND_TRIP always yields a Document; narrow it for the type checker.
    assert isinstance(doc, yamlrocks.YAMLRocksDocument)
    benchmark(doc.to_yaml)


# -- Includes (Home Assistant-style split configuration) ---------------------


@pytest.fixture(scope="session")
def include_tree(tmp_path_factory):
    """Build a root config that ``!include``s 200 small files, once per session."""
    root_dir = tmp_path_factory.mktemp("config")
    pkgs = root_dir / "packages"
    pkgs.mkdir()
    lines = []
    for i in range(200):
        name = f"pkg_{i:04d}"
        (pkgs / f"{name}.yaml").write_text(
            f"name: {name}\n"
            f"enabled: true\n"
            f"settings:\n  retries: {i % 5}\n  timeout: {10 + i}\n"
            f"items:\n  - a\n  - b\n  - c\n"
        )
        lines.append(f"{name}: !include packages/{name}.yaml")
    root = ("\n".join(lines) + "\n").encode()
    return root, str(root_dir)


def test_includes(benchmark, include_tree):
    """Resolve a configuration split across 200 included files."""
    root, include_dir = include_tree
    benchmark(
        lambda: yamlrocks.loads(
            root, option=yamlrocks.OPT_INCLUDES, include_dir=include_dir
        )
    )


# -- Custom tags (passthrough) -----------------------------------------------

_PASSTHROUGH_TAGS = b"items:\n" + b"".join(
    b"  - !vec [%d, %d, %d]\n" % (i, i + 1, i + 2) for i in range(300)
)


def test_loads_passthrough_tags(benchmark):
    """Custom-tagged nodes returned as ``YAMLRocksTag`` objects (the passthrough path)."""
    benchmark(yamlrocks.loads, _PASSTHROUGH_TAGS, option=yamlrocks.OPT_PASSTHROUGH_TAG)
