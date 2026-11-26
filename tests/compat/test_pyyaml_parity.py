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


# -- OPT_PYYAML_COMPAT: single-document loads ----------------------------------

# Streams on both sides of PyYAML's "expected a single document" rule: a second
# `---` (even an empty trailing one, or after a lone `\r` line break), a
# directive opening another document, and a bare document after `...` all
# count; a closing `...` or comments do not. A second document that is itself
# broken still reports as the extra document, as in PyYAML.
_STREAMS = [
    "a: 1\n---\nb: 2\n",
    "a: 1\n---\n",
    "a: 1\n--- # comment\n",
    "--- a\n--- b\n",
    "---\n---\n",
    "a: 1\n...\n---\n",
    "a\n...\nb\n",
    "%YAML 1.1\n---\na\n...\n%YAML 1.1\n---\nb\n",
    "a: 1\r\n---\r\nb\r\n",
    "a: 1\r---\rb: 2\r",
    "a: 1\n---\n[unterminated\n",
    "a: 1\n---\nb: 'x\n",
    "a: 1\n--- 'x\n",
    "a: [\n  1\n]\n---\n[unterminated\n",
    "a\n---\n*missing\n",
    "a: 1\n---\n- a\nb: c\n",
    "a: |\n  text\n---\n",
    "a: 1\n...\n",
    "a: 1\n...\n...\n",
    "a\n...\n# trailing comment\n",
    "# comment\n---\na\n",
    "---\n",
    "",
]

# The fast path, the AST path behind its strict gate (annotated, or tags with
# OPT_REJECT_COMPLEX_KEYS), and the ungated tag-resolving AST path each decide
# this on their own, so check every one.
_COMPAT_LOAD_MODES = {
    "default": yamlrocks.OPT_PYYAML_COMPAT,
    "annotated": yamlrocks.OPT_PYYAML_COMPAT | yamlrocks.OPT_ANNOTATED,
    "tags": yamlrocks.OPT_PYYAML_COMPAT | yamlrocks.OPT_SECRETS,
    "tags_strict": yamlrocks.OPT_PYYAML_COMPAT
    | yamlrocks.OPT_SECRETS
    | yamlrocks.OPT_REJECT_COMPLEX_KEYS,
}


def _location_of_second_document(load, stream: str) -> tuple[int, int] | None:
    """Where `load` reports a second document in `stream` (1-based line and
    column), or None when it accepts the stream as a single document."""
    try:
        load(stream)
    except yaml.YAMLError as err:
        return (err.problem_mark.line + 1, err.problem_mark.column + 1)
    except yamlrocks.YAMLRocksParseError as err:
        assert "expected a single document" in str(err)
        return (err.line, err.column)
    return None


@pytest.mark.parametrize("mode", _COMPAT_LOAD_MODES)
@pytest.mark.parametrize("stream", _STREAMS)
def test_pyyaml_compat_single_document_matches_pyyaml(stream, mode):
    """Under OPT_PYYAML_COMPAT, loads raises on exactly the streams PyYAML's
    safe_load rejects for holding more than one document, at the same spot."""
    option = _COMPAT_LOAD_MODES[mode]

    ours = _location_of_second_document(
        lambda text: yamlrocks.loads(text, option=option), stream
    )
    theirs = _location_of_second_document(yaml.safe_load, stream)

    assert ours == theirs


def test_loads_without_compat_keeps_the_first_document():
    """Without OPT_PYYAML_COMPAT, loads still returns the first document of a
    multi-document stream, as documented."""
    assert yamlrocks.loads("a: 1\n---\nb: 2\n") == {"a": 1}


# -- OPT_PYYAML_COMPAT: flow collections at or below the block indent ----------

# Inside a flow collection the spec requires every line to be indented past the
# enclosing block (test-suite case 9C9N); PyYAML does not check. A closing
# bracket at the key's column is the common case, but PyYAML accepts the items
# there too, so compat accepts all of it.
_LENIENT_FLOW = [
    "command: [\n  '/bin/bash',\n  '-c',\n  'echo hello'\n]\n",
    "a: [\nx,\ny\n]\n",
    "a:\n  b: [\n  1\n  ]\n",
    "a: {\nk: v\n}\n",
    "- [\n1,\n2\n]\n",
    "a: {x: [\n1\n]}\n",
    # The same rule right after a plain item, which consumes the line break
    # itself, and inside quoted and plain scalars continued at the block indent.
    "key: [\n  a,\n  b\n]\n",
    "key: {\n  k: v\n}\n",
    "a:\n  b: [\n    x\n  ]\n",
    "key: [a\nb]\n",
    "key: ['a\nb']\n",
    'key: ["a\nb"]\n',
]

# The fast path, the AST paths, and round-trip each build their own parser.
_COMPAT_FLOW_MODES = {
    "default": yamlrocks.OPT_PYYAML_COMPAT,
    "annotated": yamlrocks.OPT_PYYAML_COMPAT | yamlrocks.OPT_ANNOTATED,
    "tags": yamlrocks.OPT_PYYAML_COMPAT | yamlrocks.OPT_SECRETS,
    "round_trip": yamlrocks.OPT_PYYAML_COMPAT | yamlrocks.OPT_ROUND_TRIP,
}


@pytest.mark.parametrize("mode", _COMPAT_FLOW_MODES)
@pytest.mark.parametrize("source", _LENIENT_FLOW)
def test_pyyaml_compat_accepts_flow_at_the_block_indent(source, mode):
    """Under OPT_PYYAML_COMPAT, a flow collection whose lines sit at or below
    the enclosing block's indent loads like PyYAML, on every load path."""
    loaded = yamlrocks.loads(source, option=_COMPAT_FLOW_MODES[mode])

    if isinstance(loaded, yamlrocks.YAMLRocksDocument):
        assert loaded.to_yaml() == source.encode()
        loaded = loaded.to_dict()
    assert loaded == yaml.safe_load(source)


@pytest.mark.parametrize("source", _LENIENT_FLOW)
def test_flow_at_the_block_indent_stays_an_error_without_compat(source):
    """Without OPT_PYYAML_COMPAT the spec rule holds (test-suite case 9C9N)."""
    with pytest.raises(
        yamlrocks.YAMLRocksParseError,
        match=r"flow content must be|quoted scalar must be indented",
    ):
        yamlrocks.loads(source)


@pytest.mark.parametrize(
    "source",
    [
        # A comment line may sit at any indentation.
        "key: [a,\n# note\n  b]\n",
        "key: [a\n# note\n  ]\n",
        # A top-level flow collection has no block to be indented past.
        "[\na,\nb\n]\n",
        # Indented past the block, as the spec asks.
        "key: [\n  a,\n  b\n  ]\n",
        "key: [a\n  b]\n",
        "key: ['a\n  b']\n",
    ],
)
def test_flow_layouts_the_spec_allows_stay_valid(source):
    """The strict flow-indentation check only rejects what the spec rejects."""
    assert yamlrocks.loads(source) == yaml.safe_load(source)


def test_pyyaml_compat_still_rejects_an_unclosed_flow_collection():
    """Dropping the indent check does not let a flow collection swallow the rest
    of the document: one that never closes is still an error, as in PyYAML."""
    source = "a: [1,\nb: 2\n"
    with pytest.raises(yaml.YAMLError):
        yaml.safe_load(source)
    with pytest.raises(yamlrocks.YAMLRocksParseError):
        yamlrocks.loads(source, option=yamlrocks.OPT_PYYAML_COMPAT)


@pytest.mark.parametrize("marker", ["---", "..."])
@pytest.mark.parametrize("item_indent", ["", "  "])
def test_document_marker_inside_a_flow_collection_is_an_error(marker, item_indent):
    """A document marker inside an open flow collection is rejected, as PyYAML
    rejects it, with or without compat: it is never folded into a plain scalar
    (which would load `[x, ---]` as the item `x ---`)."""
    source = f"a: [\n{item_indent}x\n{marker}\n]\n"
    with pytest.raises(yaml.YAMLError):
        yaml.safe_load(source)

    for option in (0, yamlrocks.OPT_PYYAML_COMPAT):
        with pytest.raises(yamlrocks.YAMLRocksParseError):
            yamlrocks.loads(source, option=option)


@pytest.mark.parametrize("all_documents", [False, True])
def test_pyyaml_compat_upgrade_normalizes_a_lenient_flow_layout(all_documents):
    """Upgrading stamps `%YAML 1.2`, so a layout only compat reads is re-emitted
    canonically rather than verbatim: the upgraded output reloads strictly."""
    source = "command: [\n  'a',\n  'b'\n]\n# kept\n"
    option = (
        yamlrocks.OPT_PYYAML_COMPAT
        | yamlrocks.OPT_ROUND_TRIP
        | yamlrocks.OPT_UPGRADE_1_1
    )

    if all_documents:
        [doc] = yamlrocks.loads_all(source, option=option)
    else:
        doc = yamlrocks.loads(source, option=option)
    upgraded = doc.to_yaml()

    assert upgraded.startswith(b"%YAML 1.2\n")
    assert b"# kept" in upgraded
    assert yamlrocks.loads(upgraded) == {"command": ["a", "b"]}


def test_pyyaml_compat_upgrade_saves_the_normalized_layout(tmp_path):
    """Normalizing a lenient-only layout is a rewrite, like a 1.1 scalar the
    upgrade rewrites, so save() without a destination writes it back."""
    path = tmp_path / "config.yaml"
    path.write_text("command: [\n  'a',\n  'b'\n]\n")

    doc = yamlrocks.load(
        str(path),
        option=yamlrocks.OPT_PYYAML_COMPAT
        | yamlrocks.OPT_ROUND_TRIP
        | yamlrocks.OPT_UPGRADE_1_1,
    )

    assert doc.save() == [str(path)]
    assert yamlrocks.load(str(path)) == {"command": ["a", "b"]}


def test_pyyaml_compat_upgrade_normalizes_a_lenient_included_file(tmp_path):
    """An included file with a lenient-only layout is rewritten by the upgrade
    too, so the whole include graph reloads strictly. A valid file is left as is."""
    root = tmp_path / "configuration.yaml"
    root.write_text("lenient: !include lenient.yaml\nvalid: !include valid.yaml\n")
    (tmp_path / "lenient.yaml").write_text("values: [\n  'a',\n  'b'\n]\n")
    (tmp_path / "valid.yaml").write_text("values: [\n  'c'\n  ]\n")

    doc = yamlrocks.load(
        str(root),
        option=yamlrocks.OPT_PYYAML_COMPAT
        | yamlrocks.OPT_ROUND_TRIP
        | yamlrocks.OPT_UPGRADE_1_1
        | yamlrocks.OPT_INCLUDES,
    )

    assert doc.save() == [str(tmp_path / "lenient.yaml")]
    assert (tmp_path / "valid.yaml").read_text() == "values: [\n  'c'\n  ]\n"
    assert yamlrocks.load(str(root), option=yamlrocks.OPT_INCLUDES) == {
        "lenient": {"values": ["a", "b"]},
        "valid": {"values": ["c"]},
    }


def test_pyyaml_compat_upgrade_keeps_a_valid_layout_verbatim():
    """A compat source that is valid YAML anyway still upgrades byte-for-byte
    (plus the `%YAML 1.2` stamp); only lenient-only layouts are re-emitted."""
    source = "command: [\n  'a'\n  ]\n"
    doc = yamlrocks.loads(
        source,
        option=yamlrocks.OPT_PYYAML_COMPAT
        | yamlrocks.OPT_ROUND_TRIP
        | yamlrocks.OPT_UPGRADE_1_1,
    )

    assert doc.to_yaml() == b"%YAML 1.2\n---\n" + source.encode()


def test_pyyaml_compat_flow_leniency_reaches_included_files(tmp_path):
    """Included files and secrets.yaml are read in the same dialect as the root,
    so a lenient flow layout in either loads under OPT_PYYAML_COMPAT."""
    (tmp_path / "part.yaml").write_text("cmd: [\n  a,\n  b\n]\n")
    (tmp_path / "secrets.yaml").write_text("token: [\n  s1\n]\n")

    loaded = yamlrocks.loads(
        "x: !include part.yaml\ny: !secret token\n",
        option=yamlrocks.OPT_PYYAML_COMPAT
        | yamlrocks.OPT_INCLUDES
        | yamlrocks.OPT_SECRETS,
        include_dir=tmp_path,
    )

    assert loaded == {"x": {"cmd": ["a", "b"]}, "y": ["s1"]}
