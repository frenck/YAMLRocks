"""The ``YAMLRocksNode`` handle: uniform metadata access in round-trip mode.

Item access (``doc["a"]``) returns plain values for scalars and live views for
containers, which leaves nowhere to hang a scalar's comment, line, or style.
``YAMLRocksDocument.node`` solves that: it returns a ``YAMLRocksNode`` cursor that yields another
``YAMLRocksNode`` for *every* child - scalars included - so comments, source location,
style, anchor, and tag stay reachable down to individual leaves.

Comment placement follows YAML's own rules: for a mapping pair the "before"
comment sits above the key while the inline comment trails the value, so
``comment_before`` targets the key and ``comment``/``comment_after`` the value.
"""

from __future__ import annotations

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP

CONFIG = b"""\
server:
  host: localhost
  port: 8080  # the http port
  tags: [web, edge]
ports:
  - 80   # http
  - 443  # https
"""


def load(src: bytes = CONFIG):
    return yamlrocks.loads(src, option=RT)


# -- Cursor and navigation ---------------------------------------------------


def test_node_is_returned_for_scalars():
    """Indexing a YAMLRocksNode yields a YAMLRocksNode even when the value is a scalar."""
    port = load().node["server"]["port"]
    assert type(port).__name__ == "YAMLRocksNode"
    assert port.value == 8080


def test_root_node_value_is_whole_document():
    """The root cursor's value is the resolved document."""
    doc = load()
    assert doc.node.value == doc.to_dict()


def test_sequence_indexing():
    """A YAMLRocksNode indexes into sequences by position."""
    assert load().node["ports"][1].value == 443


def test_missing_key_raises_keyerror():
    """Indexing an absent mapping key raises KeyError."""
    with pytest.raises(KeyError):
        load().node["server"]["nope"]


def test_indexing_a_scalar_raises():
    """A scalar has no children to index into."""
    with pytest.raises((KeyError, IndexError)):
        load().node["server"]["port"]["x"]


def test_view_exposes_node_cursor():
    """A YAMLRocksDocumentView bridges to a YAMLRocksNode at the same path via ``.node``."""
    doc = load()
    assert doc["server"].node["port"].value == 8080


# -- Source location ---------------------------------------------------------


def test_line_and_column_are_one_based():
    """``port: 8080`` sits on line 3; its value starts at column 9."""
    port = load().node["server"]["port"]
    assert port.line == 3
    assert port.column == 9


def test_file_is_none_without_includes():
    """A single-source document reports no per-node file."""
    assert load().node["server"]["port"].file is None


def test_offset_and_end_offset_slice_exact_source():
    """`offset`/`end_offset` give a node's exact source byte range.

    The range is precise even for quoted scalars (the closing quote is included),
    where a position derived from the post-scan text would be off — so slicing
    `source[node.offset:node.end_offset]` reproduces the verbatim source token.
    """
    src = b'name: hello\nquoted: "two words"\nnum: 42\nitems:\n  - a\n  - bb\n'
    root = yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP).node
    assert src[root["name"].offset : root["name"].end_offset] == b"hello"
    # The quoted scalar's extent spans both quotes (exact, not approximate).
    q = root["quoted"]
    assert src[q.offset : q.end_offset] == b'"two words"'
    assert src[root["num"].offset : root["num"].end_offset] == b"42"
    # A collection spans to the furthest end of any child.
    items = root["items"]
    assert src[items.offset : items.end_offset] == b"- a\n  - bb"


def test_file_tracks_included_source(tmp_path):
    """With includes, a node reports the file it was loaded from."""
    (tmp_path / "main.yaml").write_text("data: !include child.yaml\n")
    (tmp_path / "child.yaml").write_text("key: value\n")
    doc = yamlrocks.load(
        str(tmp_path / "main.yaml"), option=RT | yamlrocks.OPT_INCLUDES
    )
    assert doc.node["data"]["key"].file.endswith("child.yaml")


# -- Comments: reading -------------------------------------------------------


def test_read_inline_comment():
    """``comment`` is the inline comment trailing the value, bare of ``#``."""
    assert load().node["server"]["port"].comment == "the http port"


def test_inline_comment_absent_is_none():
    """A value with no inline comment reports None."""
    assert load().node["server"]["host"].comment is None


def test_read_sequence_item_comment():
    """Sequence items carry their own inline comment."""
    ports = load().node["ports"]
    assert ports[0].comment == "http"
    assert ports[1].comment == "https"


def test_comment_before_targets_the_key():
    """``comment_before`` reads the standalone comment above a mapping key."""
    doc = load(b"# leading note\nkey: value\n")
    assert doc.node["key"].comment_before == "leading note"


def test_comment_before_absent_is_none():
    """A key with no comment above it reports None."""
    assert load().node["server"]["host"].comment_before is None


# -- Comments: writing -------------------------------------------------------


def test_set_inline_comment_round_trips():
    """Setting ``comment`` re-emits the value with the new trailing comment."""
    doc = load()
    doc.node["server"]["port"].comment = "now uses TLS"
    out = doc.to_yaml().decode()
    assert "port: 8080 # now uses TLS" in out
    assert "the http port" not in out


def test_clear_inline_comment():
    """Setting ``comment`` to None removes the trailing comment."""
    doc = load()
    doc.node["server"]["port"].comment = None
    assert "#" not in doc.to_yaml().decode().split("port: 8080")[1].splitlines()[0]


def test_set_comment_before_adds_a_line_above_the_key():
    """Setting ``comment_before`` emits a standalone comment above the key."""
    doc = load(b"key: value\n")
    doc.node["key"].comment_before = "explain the key"
    assert doc.to_yaml().decode() == "# explain the key\nkey: value\n"


def test_set_multiline_comment_before():
    """A multi-line string becomes one comment line per line."""
    doc = load(b"key: value\n")
    doc.node["key"].comment_before = "line one\nline two"
    assert doc.to_yaml().decode() == "# line one\n# line two\nkey: value\n"


def test_comment_before_on_first_key_replaces_leading_comment():
    """The leading comment lives above the first key; setting it replaces it."""
    doc = load(b"# old leading\nkey: value\nother: 2\n")
    assert doc.node["key"].comment_before == "old leading"
    doc.node["key"].comment_before = "new leading"
    assert doc.to_yaml().decode() == "# new leading\nkey: value\nother: 2\n"


# -- Value editing -----------------------------------------------------------


def test_set_value_writes_through():
    """Setting ``value`` updates the node in place."""
    doc = load()
    doc.node["server"]["port"].value = 8443
    assert doc.node["server"]["port"].value == 8443
    assert "port: 8443" in doc.to_yaml().decode()


def test_set_value_preserves_inline_comment():
    """Replacing a value keeps the comment that trailed it."""
    doc = load()
    doc.node["server"]["port"].value = 8443
    assert doc.node["server"]["port"].comment == "the http port"


def test_set_value_on_container():
    """A container value can be replaced wholesale."""
    doc = load()
    doc.node["server"].value = {"only": "this"}
    assert doc.node["server"].value == {"only": "this"}


# -- Style, anchor, tag ------------------------------------------------------


def test_scalar_style_names():
    """``style`` reports a scalar's quoting style."""
    doc = load(b"a: plain\nb: \"double\"\nc: 'single'\n")
    assert doc.node["a"].style == "plain"
    assert doc.node["b"].style == "double"
    assert doc.node["c"].style == "single"


def test_block_scalar_styles():
    """Literal and folded block scalars report their style."""
    doc = load(b"lit: |\n  hello\nfold: >\n  world\n")
    assert doc.node["lit"].style == "literal"
    assert doc.node["fold"].style == "folded"


def test_collection_styles():
    """Flow and block collections report their layout."""
    doc = load(b"flow: [1, 2]\nblock:\n  - 1\n  - 2\n")
    assert doc.node["flow"].style == "flow"
    assert doc.node["block"].style == "block"


def test_flow_style_survives_re_emit():
    """A flow collection stays flow after the document is re-emitted."""
    doc = load(b"tags: [web, edge]\nport: 8080\n")
    doc.node["port"].value = 9090
    assert b"[web, edge]" in doc.to_yaml()


def test_anchor_and_tag_are_readable():
    """``anchor`` and ``tag`` expose a node's markup."""
    doc = load(b"base: &b\n  x: 1\nref: *b\ntagged: !!str 5\n")
    assert doc.node["base"].anchor == "b"
    assert doc.node["tagged"].tag in ("!!str", "tag:yaml.org,2002:str")


def test_anchor_absent_is_none():
    """A node without an anchor reports None."""
    assert load().node["server"]["port"].anchor is None


# -- Anchors: discovery, navigation, detach ----------------------------------

ANCHORED = b"""\
defaults: &d
  retries: 3
  timeout: 30  # default
prod:
  <<: *d
  timeout: 60
staging: *d
"""


def test_document_anchors_lists_definitions():
    """``YAMLRocksDocument.anchors`` maps each anchor name to its defining YAMLRocksNode."""
    anchors = load(ANCHORED).anchors
    assert set(anchors) == {"d"}
    assert anchors["d"].value == {"retries": 3, "timeout": 30}


def test_anchors_is_empty_without_anchors():
    """A document with no anchors has an empty mapping."""
    assert load(b"a: 1\n").anchors == {}


def test_anchors_includes_one_on_a_mapping_key():
    """An anchor on a mapping key is discoverable, like one on a value."""
    doc = load(b"&kanchor key: value\nother: &vanchor 1\n")
    assert set(doc.anchors) == {"kanchor", "vanchor"}
    # The key anchor's node is the key scalar itself.
    assert doc.anchors["kanchor"].value == "key"
    assert doc.anchors["kanchor"].anchor == "kanchor"


def test_aliases_of_a_key_anchor_are_found():
    """A key anchor's ``aliases`` list the ``*name`` nodes referencing it."""
    doc = load(b"&kanchor key: value\nuse: *kanchor\nalso: *kanchor\n")
    assert len(doc.anchors["kanchor"].aliases) == 2


def test_alias_to_a_key_anchor_resolves():
    """An alias pointing at a key anchor resolves to that key's value."""
    assert yamlrocks.loads(b"&kanchor key: value\nuse: *kanchor\n") == {
        "key": "value",
        "use": "key",
    }


def test_is_alias_detects_aliases():
    """``is_alias`` is True only for ``*name`` nodes."""
    doc = load(ANCHORED)
    assert doc.node["staging"].is_alias is True
    assert doc.node["defaults"].is_alias is False


def test_target_points_at_the_definition():
    """An alias's ``target`` is the anchor-defining YAMLRocksNode."""
    doc = load(ANCHORED)
    target = doc.node["staging"].target
    assert target is not None
    assert target.anchor == "d"
    assert target.value == {"retries": 3, "timeout": 30}


def test_target_is_none_for_non_alias():
    """A non-alias node has no target."""
    assert load(ANCHORED).node["defaults"].target is None


def test_aliases_finds_uses_of_a_definition():
    """A definition's ``aliases`` are the ``*name`` nodes referencing it."""
    doc = load(ANCHORED)
    uses = doc.node["defaults"].aliases
    assert len(uses) == 2  # the `<<: *d` merge and `staging: *d`
    assert all(node.is_alias for node in uses)


def test_aliases_empty_for_plain_node():
    """A node that defines no anchor has no aliases."""
    assert load(ANCHORED).node["prod"].aliases == []


def test_indexing_an_alias_follows_transparently():
    """Indexing an alias reaches into the anchor it points at."""
    assert load(ANCHORED).node["staging"]["retries"].value == 3


def test_edit_through_alias_hits_shared_definition():
    """A write through a followed alias changes the shared anchor."""
    doc = load(ANCHORED)
    doc.node["staging"]["retries"].value = 99
    assert doc.node["defaults"]["retries"].value == 99


def test_detach_makes_an_independent_copy():
    """``detach`` replaces an alias with a copy that edits independently."""
    doc = load(ANCHORED)
    doc.node["staging"].detach()
    assert doc.node["staging"].is_alias is False

    doc.node["staging"]["retries"].value = 7
    assert doc.node["staging"]["retries"].value == 7
    assert doc.node["defaults"]["retries"].value == 3  # original untouched


def test_detach_preserves_styles_and_comments():
    """A detached copy keeps the anchored node's comments and the anchor stays."""
    doc = load(ANCHORED)
    doc.node["staging"].detach()
    out = doc.to_yaml().decode()
    assert "defaults: &d" in out  # the definition keeps its anchor
    assert out.count("# default") == 2  # comment copied into the detached block
    assert "staging: *d" not in out  # the alias is gone


def test_detach_on_non_alias_raises():
    """``detach`` is only valid on an alias node."""
    with pytest.raises(TypeError, match="alias"):
        load(ANCHORED).node["defaults"].detach()


def test_detached_clone_expands_inner_aliases():
    """Aliases nested inside a detached block are expanded to copies."""
    src = b"a: &a 1\nbox: &box\n  inner: *a\nuse: *box\n"
    doc = yamlrocks.loads(src, option=RT)
    doc.node["use"].detach()
    # `use` now holds a concrete {inner: 1}, independent of &a.
    assert doc.node["a"].value == 1  # the original anchor is untouched
    assert doc.node["use"]["inner"].value == 1
    assert doc.node["use"].is_alias is False


# -- Anchors: creation -------------------------------------------------------


def test_create_anchor_and_alias():
    """Mark a node as an anchor, then point another node at it."""
    doc = load(b"defaults:\n  retries: 3\nprod:\n  retries: 5\n")
    doc.node["defaults"].anchor = "d"
    doc.node["prod"].make_alias("d")
    assert doc.to_yaml() == b"defaults: &d\n  retries: 3\nprod: *d\n"


def test_created_alias_resolves_on_reload():
    """A created `&`/`*` pair round-trips and resolves like a parsed one."""
    doc = load(b"base:\n  x: 1\nref: replaced\n")
    doc.node["base"].anchor = "b"
    doc.node["ref"].make_alias("b")
    reloaded = yamlrocks.loads(doc.to_yaml(), option=RT)
    assert reloaded.node["ref"].is_alias is True
    assert reloaded.node["ref"].value == {"x": 1}


def test_clear_anchor():
    """Setting ``anchor`` to None removes the `&name` marker."""
    doc = load(b"base: &b\n  x: 1\n")
    assert doc.node["base"].anchor == "b"
    doc.node["base"].anchor = None
    assert doc.node["base"].anchor is None
    assert b"&b" not in doc.to_yaml()


def test_reassigning_same_anchor_name_to_its_node_is_allowed():
    """Setting a node's own existing anchor name again is not a conflict."""
    doc = load(b"base: &b\n  x: 1\n")
    doc.node["base"].anchor = "b"  # same name, same node
    assert doc.node["base"].anchor == "b"


def test_duplicate_anchor_name_rejected():
    """Two nodes cannot share an anchor name (would emit two `&name`)."""
    doc = load(b"a: 1\nb: 2\n")
    doc.node["a"].anchor = "x"
    with pytest.raises(ValueError, match="already used"):
        doc.node["b"].anchor = "x"


def test_empty_anchor_name_rejected():
    """An anchor name cannot be empty."""
    doc = load(b"a: 1\n")
    with pytest.raises(ValueError, match="empty"):
        doc.node["a"].anchor = ""


@pytest.mark.parametrize("name", ["a b", "a\nb", "[x]", "tag,s", "{m}"])
def test_invalid_anchor_name_rejected(name):
    """An anchor name with whitespace, a break, or a flow indicator is rejected.

    Such a name would split when the `&name` is emitted and read back, corrupting
    the document (`anchor = "a b"` -> `&a b value`, reloading as `b value`).
    """
    doc = load(b"a: 1\n")
    with pytest.raises(ValueError, match="invalid anchor name"):
        doc.node["a"].anchor = name


def test_alias_to_undefined_anchor_rejected():
    """An alias needs an existing anchor to point at."""
    doc = load(b"a: 1\nb: 2\n")
    with pytest.raises(ValueError, match="no anchor"):
        doc.node["b"].make_alias("ghost")


def test_alias_forward_reference_rejected():
    """The anchor must precede the alias in document order."""
    doc = load(b"first: 1\nsecond: 2\n")
    doc.node["second"].anchor = "s"
    with pytest.raises(ValueError, match="before"):
        doc.node["first"].make_alias("s")  # first comes before second


def test_alias_to_ancestor_rejected_as_cycle():
    """An alias cannot point at its own ancestor: that is a cycle, since the
    anchored container holds the alias that would re-insert it forever."""
    doc = load(b"root: &r\n  child: 1\n")
    with pytest.raises(ValueError, match="cycle"):
        doc.node["root"]["child"].make_alias("r")


def test_make_alias_replaces_value():
    """make_alias replaces the node's current value with the alias."""
    doc = load(b"base: &b\n  x: 1\nother:\n  y: 2\n")
    doc.node["other"].make_alias("b")
    assert doc.node["other"].is_alias is True
    assert doc.node["other"].value == {"x": 1}
