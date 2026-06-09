"""Cross-check parsing against PyYAML for common documents.

YAMLRocks targets the YAML 1.2 core schema while PyYAML implements YAML 1.1, so the
comparisons here deliberately avoid cases where the schemas diverge (``yes``/``no``
booleans, sexagesimal numbers, octal literals, etc.).
"""

from __future__ import annotations

import pytest

import yamlrocks

yaml = pytest.importorskip("yaml")


COMMON_DOCUMENTS = [
    b"key: value",
    b"a: 1\nb: 2\nc: 3",
    b"nested:\n  inner:\n    value: 42",
    b"list:\n  - 1\n  - 2\n  - 3",
    b"mixed:\n  - name: alice\n    age: 30\n  - name: bob\n    age: 25",
    b"flow_map: {a: 1, b: 2}",
    b"flow_list: [1, 2, 3]",
    b"types:\n  i: 42\n  f: 3.14\n  s: hello\n  t: true\n  f2: false\n  n: null",
    b"quoted:\n  single: 'value'\n  double: \"value\"",
    b"empty_map: {}\nempty_list: []",
]


@pytest.mark.parametrize("doc", COMMON_DOCUMENTS)
def test_loads_matches_pyyaml(doc):
    """Loading a common document matches PyYAML's safe_load result."""
    assert yamlrocks.loads(doc) == yaml.safe_load(doc)


@pytest.mark.parametrize(
    "obj",
    [
        {"key": "value"},
        {"a": 1, "b": 2},
        {"nested": {"inner": [1, 2, 3]}},
        [{"name": "alice"}, {"name": "bob"}],
        {"types": {"i": 42, "f": 3.14, "s": "hello", "t": True, "n": None}},
    ],
)
def test_dump_is_loadable_by_pyyaml(obj):
    """Dumped output is loadable by PyYAML back to the original object."""
    dumped = yamlrocks.dumps(obj)
    assert yaml.safe_load(dumped) == obj


# -- OPT_PYYAML_COMPAT: byte-for-byte PyYAML boolean parity --------------------

# The scalars where PyYAML's bool set matters, in key and value position. PyYAML
# omits bare y/Y/n/N from booleans (off-spec vs YAML 1.1); OPT_PYYAML_COMPAT
# matches that exactly.
_BOOLISH = [
    "y",
    "Y",
    "n",
    "N",
    "yes",
    "Yes",
    "YES",
    "no",
    "No",
    "NO",
    "on",
    "On",
    "ON",
    "off",
    "Off",
    "OFF",
    "true",
    "True",
    "TRUE",
    "false",
    "False",
    "FALSE",
    "yEs",
    "TrUe",
    "ONOFF",
    "0777",
    "1:30",
]


@pytest.mark.parametrize("scalar", _BOOLISH)
def test_pyyaml_compat_value_matches_pyyaml(scalar):
    """A value resolves identically under OPT_PYYAML_COMPAT and PyYAML."""
    option = yamlrocks.OPT_YAML_1_1 | yamlrocks.OPT_PYYAML_COMPAT
    rust = yamlrocks.loads(f"k: {scalar}".encode(), option=option)["k"]
    pyyaml = yaml.safe_load(f"k: {scalar}")["k"]
    assert rust == pyyaml
    assert type(rust) is type(pyyaml)


def test_pyyaml_compat_keys_match_pyyaml():
    """Mapping keys resolve like PyYAML: y/n stay strings, yes/on/off coerce."""
    src = "x: 1\ny: 2\nn: 3\nyes: 4\non: 5\noff: 6\n"
    option = yamlrocks.OPT_YAML_1_1 | yamlrocks.OPT_PYYAML_COMPAT
    rust_keys = list(yamlrocks.loads(src.encode(), option=option).keys())
    pyyaml_keys = list(yaml.safe_load(src).keys())
    assert rust_keys == pyyaml_keys  # ['x', 'y', 'n', True, False]


# -- Alias identity parity: an alias is the same object as its anchor ----------


def test_alias_identity_matches_pyyaml():
    """On the annotated (object) path, *a is the same object as &a, just as
    PyYAML resolves an alias to its anchor's object rather than a copy."""
    src = b"base: &a\n  k: 1\nref: *a\n"
    pyyaml = yaml.safe_load(src)
    assert pyyaml["base"] is pyyaml["ref"]  # PyYAML shares the object

    data = yamlrocks.loads(src, option=yamlrocks.OPT_ANNOTATED)
    assert data["base"] is data["ref"]  # YAMLRocks matches that
