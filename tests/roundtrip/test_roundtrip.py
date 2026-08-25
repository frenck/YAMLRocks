"""Structure-preserving round-trip mode (``OPT_ROUND_TRIP``).

These tests verify that comments, anchors, quoting styles and formatting survive
a load → edit → emit cycle, and that unmodified content stays byte-identical.
"""

from __future__ import annotations

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP


def roundtrip(src: str) -> str:
    """Load ``src`` in round-trip mode and emit it back to text."""
    return yamlrocks.loads(src.encode(), option=RT).to_yaml().decode()


def test_load_returns_document():
    """Loading in round-trip mode returns a YAMLRocksDocument instance."""
    doc = yamlrocks.loads(b"key: value", option=RT)
    assert isinstance(doc, yamlrocks.YAMLRocksDocument)


def test_plain_mapping_is_byte_identical():
    """An unmodified plain mapping round-trips byte-for-byte."""
    src = "key: value\nother: thing\n"
    assert roundtrip(src) == src


def test_head_comment_preserved():
    """Round-trip preserves a head comment above a key."""
    src = "# top comment\nkey: value\n"
    assert roundtrip(src) == src


def test_multiple_head_comments_preserved():
    """Round-trip preserves multiple stacked head comments."""
    src = "# line one\n# line two\nkey: val\n"
    assert roundtrip(src) == src


def test_comment_between_keys_preserved():
    """Round-trip preserves a comment sitting between two keys."""
    src = "a: 1\n# between\nb: 2\n"
    assert roundtrip(src) == src


def test_nested_head_comment_preserved():
    """Round-trip preserves a head comment inside a nested mapping."""
    src = "outer:\n  # nested\n  inner: 1\n"
    assert roundtrip(src) == src


def test_inline_comment_preserved():
    """Round-trip preserves a trailing inline comment and its spacing."""
    # An unmodified round-trip is byte-for-byte, so original spacing is kept.
    assert roundtrip("key: value  # trailing\n") == "key: value  # trailing\n"


def test_foot_comment_preserved():
    """Round-trip preserves a foot comment after the last key."""
    src = "key: value\n# trailing foot\n"
    assert roundtrip(src) == src


def test_nested_final_block_foot_keeps_indent_on_edit():
    """A comment at the end of a nested final block keeps that block's indent
    after an edit, instead of flattening to column 0.

    Unmodified documents re-emit from the source cache, which hides the bug; an
    edit forces the AST path, where the trailing comment must stay owned by the
    nested block it sits in (not the document root, which emits at column 0).
    """
    doc = yamlrocks.loads(b"parent:\n  a: 1\n  b: 2\n  # end of parent\n", option=RT)
    doc["parent"]["b"] = 9
    assert doc.to_yaml() == b"parent:\n  a: 1\n  b: 9\n  # end of parent\n"


def test_sequence_comments_preserved():
    """Round-trip preserves comments attached to sequence items."""
    src = "items:\n  # about list\n  - one # first\n  - two\n"
    assert roundtrip(src) == src


def test_key_line_comment_stays_on_the_key_after_an_edit():
    """A comment after `key:` stays there when the value is a block below it.

    Unmodified documents re-emit from the source cache, which hides the
    placement; an edit forces the AST path, where the comment used to slide down
    and become a standalone line above the first child.
    """
    doc = yamlrocks.loads(b"servers: # the pool\n  - alpha\nport: 80\n", option=RT)
    doc["port"] = 8080
    assert doc.to_yaml() == b"servers: # the pool\n  - alpha\nport: 8080\n"


def test_key_line_comment_survives_an_edit_for_a_scalar_below_the_key():
    """The same for a plain scalar written under its key, where it was dropped."""
    doc = yamlrocks.loads(b"name: # explain\n  app\nport: 80\n", option=RT)
    doc["port"] = 8080
    assert doc.to_yaml() == b"name: # explain\n  app\nport: 8080\n"


def test_key_line_comment_keeps_its_alignment_padding():
    """The gap between the `:` and the `#` survives the AST path."""
    doc = yamlrocks.loads(b"servers:    # the pool\n  - alpha\nport: 80\n", option=RT)
    doc["port"] = 8080
    assert doc.to_yaml() == b"servers:    # the pool\n  - alpha\nport: 8080\n"


def test_dash_line_comment_stays_on_the_dash_after_an_edit():
    """A comment after a `-` keeps its place, with the item below it."""
    doc = yamlrocks.loads(b"- # the first one\n  a: 1\n- b: 2\n", option=RT)
    doc[1]["b"] = 9
    assert doc.to_yaml() == b"- # the first one\n  a: 1\n- b: 9\n"


def test_comments_below_a_dash_line_comment_stay_below_it():
    """Head comments written under `- # note` belong there, not above the dash."""
    src = b"- # first\n  # second\n  a: 1\n- b: 2\n"
    doc = yamlrocks.loads(src, option=RT)
    doc[1]["b"] = 9
    assert doc.to_yaml() == b"- # first\n  # second\n  a: 1\n- b: 9\n"


def test_key_line_comment_kept_when_the_value_itself_is_replaced():
    """Replacing a block value keeps the comment on the key line."""
    doc = yamlrocks.loads(b"servers: # the pool\n  - alpha\n", option=RT)
    doc.node["servers"].value = ["beta", "gamma"]
    assert doc.to_yaml().startswith(b"servers: # the pool\n")


def test_replacing_a_value_below_its_key_keeps_the_comment_there():
    """A replaced scalar stays under the key, with the comment above it."""
    doc = yamlrocks.loads(b"name: # explain\n  app\nport: 80\n", option=RT)
    doc.node["name"].value = "web"
    assert doc.to_yaml() == b"name: # explain\n  web\nport: 80\n"


def test_replacing_an_item_keeps_its_comments_in_order():
    """Replacing an item written under `- # note` keeps both comments in place.

    The head comments below the dash are emitted by the item body, so an edit
    that dropped the placement flag would re-emit them above the dash instead,
    reordering them against the comment on the dash itself.
    """
    doc = yamlrocks.loads(b"- # first\n  # second\n  a: 1\n- b: 2\n", option=RT)
    doc.node[0].value = {"a": 9}
    assert doc.to_yaml() == b"- # first\n  # second\n  a: 9\n- b: 2\n"


def test_blank_line_above_a_commented_dash_survives():
    """Section spacing before `- # note` is measured from the dash, not the item."""
    doc = yamlrocks.loads(b"- one\n\n- # note\n  two\n", option=RT)
    doc.node[0].value = "ONE"
    assert doc.to_yaml() == b"- ONE\n\n- # note\n  two\n"


def test_a_commented_dash_does_not_invent_a_leading_blank_line():
    """A blank line *inside* the entry is not re-emitted above its dash."""
    doc = yamlrocks.loads(b"- # note\n\n  two\n- x\n", option=RT)
    doc.node[1].value = "X"
    assert doc.to_yaml() == b"- # note\n  two\n- X\n"


def test_clearing_a_dash_line_comment_keeps_the_comments_below_it():
    """Clearing `- # note` keeps the head comments written under that dash."""
    doc = yamlrocks.loads(b"- # first\n  # second\n  a: 1\n- b: 2\n", option=RT)
    doc.node[0].comment = None
    assert doc.to_yaml() == b"-\n  # second\n  a: 1\n- b: 2\n"


def test_anchor_stays_on_the_key_line_beside_its_comment():
    """An anchor written before the comment stays there, not on the value's line."""
    doc = yamlrocks.loads(b"key: &a # note\n  value\nz: 1\n", option=RT)
    doc.node["z"].value = 2
    assert doc.to_yaml() == b"key: &a # note\n  value\nz: 2\n"


def test_tag_stays_on_the_key_line_beside_its_comment():
    """The same for a tag written between the `:` and the comment."""
    doc = yamlrocks.loads(
        b"key: !t # note\n  value\nz: 1\n", option=RT | yamlrocks.OPT_PASSTHROUGH_TAG
    )
    doc.node["z"].value = 2
    assert doc.to_yaml() == b"key: !t # note\n  value\nz: 2\n"


def test_section_comment_above_a_commented_dash_stays_above_it():
    """A comment above the `-` and one on it keep their own sides of the dash.

    They arrive as one run of comments, so the split between them has to be
    recorded: re-emitting the lot on either side reorders them against the
    comment written on the dash itself.
    """
    src = b"- a\n# about next\n- # inline\n  b: 1\n"
    doc = yamlrocks.loads(src, option=RT)
    assert doc.node[1].comment == "inline"
    doc.node[0].value = "A"
    assert doc.to_yaml() == b"- A\n# about next\n- # inline\n  b: 1\n"


def test_comments_on_both_sides_of_a_commented_dash_survive():
    """A comment above the dash, one on it, and one below all keep their place."""
    src = b"- a\n# above\n- # inline\n  # below\n  b: 1\n"
    doc = yamlrocks.loads(src, option=RT)
    doc.node[0].value = "A"
    assert doc.to_yaml() == b"- A\n# above\n- # inline\n  # below\n  b: 1\n"
    # The split travels with the comments when the item itself is replaced.
    doc = yamlrocks.loads(src, option=RT)
    doc.node[1].value = {"b": 9}
    assert doc.to_yaml() == b"- a\n# above\n- # inline\n  # below\n  b: 9\n"


def test_comment_before_on_an_item_lands_above_its_dash():
    """``comment_before`` replaces the head block and writes above the `-`."""
    doc = yamlrocks.loads(b"- a\n# above\n- # inline\n  b: 1\n", option=RT)
    doc.node[1].comment_before = "fresh"
    assert doc.to_yaml() == b"- a\n# fresh\n- # inline\n  b: 1\n"


def test_empty_implicit_key_introduces_from_its_own_colon():
    """An empty key (`: value`) is placed at its `:`, so it introduces from there.

    The node has no source text of its own; left at the document start (the
    obvious default) it would measure the introducer against an unrelated line,
    either missing the comment or claiming one written somewhere else entirely.
    """
    src = b"a: 1\n: # note\n  child: v\n"
    doc = yamlrocks.loads(src, option=RT)
    assert doc.to_yaml() == src
    doc.node["a"].value = 2
    assert doc.to_yaml() == b"a: 2\nnull: # note\n  child: v\n"


def test_quoting_styles_preserved():
    """Round-trip preserves single and double quoting styles."""
    src = "double: \"quoted value\"\nsingle: 'sq'\n"
    assert roundtrip(src) == src


def test_block_anchor_preserved():
    """Round-trip preserves a block anchor and its alias reference."""
    src = "base: &a\n  x: 1\nref: *a\n"
    assert roundtrip(src) == src


def test_scalar_anchor_preserved():
    """Round-trip preserves a scalar anchor and its alias reference."""
    src = "a: &x 1\nb: *x\n"
    assert roundtrip(src) == src


def test_tag_preserved():
    """Round-trip preserves a custom tag on a mapping."""
    src = "data: !mytag\n  x: 1\n"
    assert roundtrip(src) == src


def test_literal_block_preserved():
    """Round-trip keeps the literal block scalar style marker."""
    doc = yamlrocks.loads(b"text: |\n  line1\n  line2\n", option=RT)
    assert b"|" in doc.to_yaml()


def test_top_level_getitem():
    """Indexing a document returns the typed top-level values."""
    doc = yamlrocks.loads(b"key: value\nnum: 42\n", option=RT)
    assert doc["key"] == "value"
    assert doc["num"] == 42


def test_edit_top_level_value():
    """Editing a top-level value updates only that key on emit."""
    doc = yamlrocks.loads(b"key: value\nother: thing\n", option=RT)
    doc["key"] = "newvalue"
    out = doc.to_yaml().decode()
    assert out == "key: newvalue\nother: thing\n"


@pytest.mark.parametrize(
    "value",
    [
        "...",
        "\t",
        "\r",
        "x ",
        "0x1f",
        ".inf",
        ".nan",
        "TRUE",
        "NULL",
        "a\nb",
        "|x",
        ">x",
        "'x",
        '"x',
        "@x",
        "`x",
        ",x",
        "key:",
        "true",
        "null",
        "1.5",
        "yes",
        "no",
    ],
)
def test_edit_assigns_injection_prone_value(value):
    """Assigning an injection-prone string through the mutation API round-trips.

    The assignment path must quote by exactly the same rules as the fast encoder;
    a weaker duplicate check previously let these values emit unquoted, silently
    changing type or producing unparsable YAML (and losing sibling keys).
    """
    doc = yamlrocks.loads(b"a: 1\nb: 2\n", option=RT)
    doc["a"] = value
    reloaded = yamlrocks.loads(doc.to_yaml(), option=RT).to_dict()
    assert reloaded == {"a": value, "b": 2}


def test_edit_preserves_surrounding_comments():
    """Editing a value keeps surrounding comments intact on emit."""
    doc = yamlrocks.loads(b"# config\nname: app  # the name\nport: 8080\n", option=RT)
    doc["port"] = 9090
    out = doc.to_yaml().decode()
    assert "# config" in out
    assert "# the name" in out
    assert "port: 9090" in out


def test_explicit_document_start_preserved_on_edit():
    """An explicit `---` marker survives re-emission after an edit.

    Unmodified documents re-emit from the source cache, which masks this; a real
    Home Assistant include file (`---` then comments then `[]`) exposed it via the
    include write-back path, where each file is re-emitted from the AST.
    """
    doc = yamlrocks.loads(b"---\n# a comment\nkey: value\n", option=RT)
    doc["key"] = "changed"
    assert doc.to_yaml() == b"---\n# a comment\nkey: changed\n"


def test_no_document_start_is_not_invented():
    """A document without `---` does not gain one on re-emission."""
    doc = yamlrocks.loads(b"# c\nkey: value\n", option=RT)
    doc["key"] = "x"
    assert doc.to_yaml() == b"# c\nkey: x\n"


def test_explicit_start_byte_identical_unmodified():
    """An unmodified `---` document round-trips byte-for-byte."""
    src = b"---\n# only a comment\n[]\n"
    assert yamlrocks.loads(src, option=RT).to_yaml() == src


def test_deep_edit_mapping_and_sequence():
    """Editing nested mapping and sequence values is reflected on emit."""
    doc = yamlrocks.loads(
        b"server:\n  host: localhost\n  ports:\n    - 80\n    - 443\n", option=RT
    )
    doc["server"]["host"] = "example.com"
    doc["server"]["ports"][1] = 8443
    out = doc.to_yaml().decode()
    assert "host: example.com" in out
    assert "8443" in out


def test_add_new_key():
    """Assigning a new key adds it to the emitted document."""
    doc = yamlrocks.loads(b"a: 1\n", option=RT)
    doc["b"] = 2
    assert yamlrocks.loads(doc.to_yaml()) == {"a": 1, "b": 2}


def test_contains_and_keys():
    """Membership tests and keys() reflect the document mapping."""
    doc = yamlrocks.loads(b"a: 1\nb: 2\n", option=RT)
    assert "a" in doc
    assert "missing" not in doc
    assert list(doc.keys()) == ["a", "b"]


def test_delete_top_level_key():
    """Deleting a top-level key removes it from the emitted document."""
    doc = yamlrocks.loads(b"a: 1\nb: 2\nc: 3\n", option=RT)
    del doc["b"]
    assert doc.to_yaml() == b"a: 1\nc: 3\n"


def test_delete_nested_key():
    """Deleting a key through a nested view is reflected on emit."""
    doc = yamlrocks.loads(b"server:\n  host: localhost\n  port: 80\n", option=RT)
    del doc["server"]["port"]
    assert yamlrocks.loads(doc.to_yaml()) == {"server": {"host": "localhost"}}


def test_delete_preserves_surrounding_comments():
    """Deleting a key keeps the comments on the surrounding keys intact."""
    doc = yamlrocks.loads(
        b"# config\nname: app  # the name\nport: 8080  # the port\n", option=RT
    )
    del doc["port"]
    assert doc.to_yaml() == b"# config\nname: app  # the name\n"


def test_delete_takes_the_keys_own_comments_with_it():
    """A deleted key's own head and inline comments are removed with it."""
    doc = yamlrocks.loads(b"a: 1\n# b heading\nb: 2  # inline b\nc: 3\n", option=RT)
    del doc["b"]
    out = doc.to_yaml().decode()
    assert "# b heading" not in out
    assert "inline b" not in out
    assert out == "a: 1\nc: 3\n"


def test_delete_missing_key_raises_keyerror():
    """Deleting a key that is not present raises KeyError."""
    doc = yamlrocks.loads(b"a: 1\n", option=RT)
    with pytest.raises(KeyError):
        del doc["missing"]


def test_delete_then_add_round_trips():
    """Deleting a key then adding another round-trips, keeping comments."""
    doc = yamlrocks.loads(b"account:\n  user: me  # login\n  password: x\n", option=RT)
    del doc["account"]["password"]
    doc["account"]["keyring"] = None
    assert "# login" in doc.to_yaml().decode()
    assert yamlrocks.loads(doc.to_yaml()) == {
        "account": {"user": "me", "keyring": None}
    }


def test_delete_reflected_in_contains_and_keys():
    """After a delete, membership and keys() no longer report the key."""
    doc = yamlrocks.loads(b"a: 1\nb: 2\n", option=RT)
    del doc["a"]
    assert "a" not in doc
    assert list(doc.keys()) == ["b"]


def test_delete_sequence_item():
    """Deleting a sequence item by index removes it on emit."""
    doc = yamlrocks.loads(b"items:\n  - one\n  - two\n  - three\n", option=RT)
    del doc["items"][1]
    assert yamlrocks.loads(doc.to_yaml()) == {"items": ["one", "three"]}


def test_delete_sequence_index_out_of_range_raises():
    """Deleting an out-of-range sequence index raises IndexError."""
    doc = yamlrocks.loads(b"items:\n  - one\n", option=RT)
    with pytest.raises(IndexError):
        del doc["items"][5]


def test_view_unwrap_returns_plain():
    """A view's unwrap() returns a plain Python value."""
    doc = yamlrocks.loads(b"server:\n  host: localhost\n", option=RT)
    assert doc["server"].unwrap() == {"host": "localhost"}


def test_dumps_accepts_document():
    """dumps() accepts a YAMLRocksDocument and emits its bytes."""
    doc = yamlrocks.loads(b"a: 1\n", option=RT)
    assert yamlrocks.dumps(doc) == b"a: 1\n"


def test_multi_document_emit_after_edit():
    """Editing one document of a multi-document file re-emits all with `---`."""
    doc = yamlrocks.loads(b"a: 1\n---\nb: 2\n", option=RT)
    assert len(doc) == 2
    doc[0] = {"z": 9}
    assert doc.to_yaml() == b"z: 9\n---\nb: 2\n"


def test_delete_document_from_multi():
    """Deleting one document of a multi-document stream drops it on emit."""
    doc = yamlrocks.loads(b"a: 1\n---\nb: 2\n---\nc: 3\n", option=RT)
    assert len(doc) == 3
    del doc[1]
    assert len(doc) == 2
    assert yamlrocks.loads_all(doc.to_yaml()) == [{"a": 1}, {"c": 3}]


def test_multi_document_repr_and_len():
    """A multi-document YAMLRocksDocument reports its count via repr and len."""
    doc = yamlrocks.loads(b"a: 1\n---\nb: 2\n", option=RT)
    assert repr(doc) == "YAMLRocksDocument(documents=2)"
    assert len(doc) == 2


def test_multi_document_index_out_of_range():
    """Indexing a multi-document YAMLRocksDocument past its end raises IndexError."""
    doc = yamlrocks.loads(b"a: 1\n---\nb: 2\n", option=RT)
    with pytest.raises(IndexError):
        doc[5]


def test_multi_document_non_integer_key():
    """A non-integer key on a multi-document YAMLRocksDocument raises KeyError."""
    doc = yamlrocks.loads(b"a: 1\n---\nb: 2\n", option=RT)
    with pytest.raises(KeyError):
        doc["a"]


def test_multi_document_keys_not_available():
    """keys() on a multi-document YAMLRocksDocument raises a TypeError."""
    doc = yamlrocks.loads(b"a: 1\n---\nb: 2\n", option=RT)
    with pytest.raises(TypeError):
        doc.keys()


def test_flow_sequence_reemitted_after_edit():
    """A flow sequence keeps its `[...]` form when the document is re-emitted."""
    doc = yamlrocks.loads(b"a: [1, 2, 3]\nb: 0\n", option=RT)
    doc["b"] = 9
    assert doc.to_yaml() == b"a: [1, 2, 3]\nb: 9\n"


def test_flow_mapping_reemitted_after_edit():
    """A flow mapping keeps its `{...}` form when the document is re-emitted."""
    doc = yamlrocks.loads(b"a: {x: 1, y: 2}\nb: 0\n", option=RT)
    doc["b"] = 9
    assert doc.to_yaml() == b"a: {x: 1, y: 2}\nb: 9\n"


def test_literal_block_reemitted_after_edit():
    """A literal block scalar re-emits with its `|` marker after an edit."""
    doc = yamlrocks.loads(b"text: |\n  l1\n  l2\nz: 0\n", option=RT)
    doc["z"] = 9
    assert doc.to_yaml() == b"text: |\n  l1\n  l2\nz: 9\n"


def test_literal_block_strip_chomp_reemitted():
    """A strip-chomped literal block re-emits with its `|-` marker after edit."""
    doc = yamlrocks.loads(b"text: |-\n  l1\n  l2\nz: 0\n", option=RT)
    doc["z"] = 9
    assert doc.to_yaml() == b"text: |-\n  l1\n  l2\nz: 9\n"


def test_literal_block_keep_chomp_reemitted():
    """A keep-chomped literal block preserves trailing blanks on re-emit."""
    doc = yamlrocks.loads(b"text: |+\n  l1\n\n\nz: 0\n", option=RT)
    doc["z"] = 9
    assert doc.to_yaml() == b"text: |+\n  l1\n\n\nz: 9\n"


def test_empty_key_reemitted_after_edit():
    """A key with no value re-emits as bare `key:` after a sibling edit."""
    doc = yamlrocks.loads(b"a:\nb: 1\n", option=RT)
    doc["b"] = 9
    assert doc.to_yaml() == b"a:\nb: 9\n"


def test_nested_sequence_in_sequence_reemitted():
    """A sequence nested inside a sequence re-emits in block form after edit."""
    doc = yamlrocks.loads(b"m:\n  - - 1\n    - 2\nz: 0\n", option=RT)
    doc["z"] = 9
    assert yamlrocks.loads(doc.to_yaml()) == {"m": [[1, 2]], "z": 9}


def test_single_quote_apostrophe_reemitted():
    """A single-quoted scalar with an embedded apostrophe re-emits doubled."""
    doc = yamlrocks.loads(b"x: 'it''s'\ny: 0\n", option=RT)
    doc["y"] = 9
    assert doc.to_yaml() == b"x: 'it''s'\ny: 9\n"


def test_double_quote_reemitted_after_edit():
    """A double-quoted scalar keeps its quoting when re-emitted after an edit."""
    doc = yamlrocks.loads(b'x: "dq"\ny: 0\n', option=RT)
    doc["y"] = 9
    assert doc.to_yaml() == b'x: "dq"\ny: 9\n'


def test_top_level_tagged_mapping_reemitted():
    """A tag on the top-level mapping re-emits on its own line after an edit."""
    doc = yamlrocks.loads(b"!mytag\nx: 1\n", option=RT)
    doc["x"] = 9
    assert doc.to_yaml() == b"!mytag\nx: 9\n"


def test_block_collection_tag_and_anchor_reemitted():
    """A block mapping carrying both a tag and an anchor re-emits both."""
    doc = yamlrocks.loads(b"data: &a !mytag\n  x: 1\nz: 0\n", option=RT)
    doc["z"] = 9
    assert doc.to_yaml() == b"data: !mytag &a\n  x: 1\nz: 9\n"


def test_assign_none_emits_empty_value():
    """Assigning None emits a bare `key:` with no value (the round-trip default,
    matching how configurations are usually hand-written)."""
    doc = yamlrocks.loads(b"x: 1\n", option=RT)
    doc["x"] = None
    assert doc.to_yaml() == b"x:\n"


def test_assign_none_with_tilde_flag_emits_tilde():
    """OPT_NULL_AS_TILDE renders an assigned None as `~` on re-emission."""
    doc = yamlrocks.loads(b"x: 1\n", option=RT | yamlrocks.OPT_NULL_AS_TILDE)
    doc["x"] = None
    assert doc.to_yaml() == b"x: ~\n"
    new = yamlrocks.loads(b"a: 1\n", option=RT | yamlrocks.OPT_NULL_AS_TILDE)
    new["b"] = None
    assert new.to_yaml() == b"a: 1\nb: ~\n"


def test_assign_none_with_keyword_flag_emits_null():
    """OPT_NULL_AS_KEYWORD renders an assigned None as the explicit `null`."""
    doc = yamlrocks.loads(b"x: 1\n", option=RT | yamlrocks.OPT_NULL_AS_KEYWORD)
    doc["x"] = None
    assert doc.to_yaml() == b"x: null\n"


def test_loaded_nulls_keep_their_form_when_a_sibling_is_edited():
    """The null style applies only to edited-in nulls: loaded nulls re-emit in
    their original form so an untouched value stays byte-for-byte, even when a
    sibling edit forces re-emission and a non-default null style is active."""
    src = b"a:\nb: null\nc: ~\nd: 1\n"
    doc = yamlrocks.loads(src, option=RT | yamlrocks.OPT_NULL_AS_TILDE)
    doc["d"] = 2
    assert doc.to_yaml() == b"a:\nb: null\nc: ~\nd: 2\n"


def test_null_style_flags_mutually_exclusive_on_load():
    """Both null-style flags together is a ValueError at load time too."""
    with pytest.raises(ValueError, match="mutually exclusive"):
        yamlrocks.loads(
            b"x: 1\n",
            option=RT | yamlrocks.OPT_NULL_AS_KEYWORD | yamlrocks.OPT_NULL_AS_TILDE,
        )


def test_assign_bool_emits_keyword():
    """Assigning a bool emits the canonical true/false keyword."""
    doc = yamlrocks.loads(b"x: 0\n", option=RT)
    doc["x"] = True
    assert doc.to_yaml() == b"x: true\n"


def test_assign_float_emits_decimal():
    """Assigning a float emits its decimal form."""
    doc = yamlrocks.loads(b"x: 0\n", option=RT)
    doc["x"] = 3.14
    assert doc.to_yaml() == b"x: 3.14\n"


def test_assign_infinity_emits_inf():
    """Assigning positive and negative infinity emits the .inf spelling."""
    doc = yamlrocks.loads(b"x: 0\ny: 0\n", option=RT)
    doc["x"] = float("inf")
    doc["y"] = float("-inf")
    out = doc.to_yaml()
    assert b"x: .inf" in out
    assert b"y: -.inf" in out


def test_assign_nan_emits_nan():
    """Assigning NaN emits the .nan spelling."""
    doc = yamlrocks.loads(b"x: 0\n", option=RT)
    doc["x"] = float("nan")
    assert doc.to_yaml() == b"x: .nan\n"


def test_assign_string_needing_quotes_is_quoted():
    """Assigning a string that resolves to a keyword is double-quoted by default."""
    doc = yamlrocks.loads(b"x: 0\n", option=RT)
    doc["x"] = "yes"
    assert doc.to_yaml() == b'x: "yes"\n'


def test_assign_string_with_single_quotes_flag():
    """OPT_SINGLE_QUOTES makes an assigned forced-quote string single-quoted."""
    doc = yamlrocks.loads(b"x: 0\n", option=RT | yamlrocks.OPT_SINGLE_QUOTES)
    doc["x"] = "yes"
    assert doc.to_yaml() == b"x: 'yes'\n"


def test_assign_list_emits_block_sequence():
    """Assigning a list emits a block sequence under the key."""
    doc = yamlrocks.loads(b"x: 0\n", option=RT)
    doc["x"] = [1, 2, "three"]
    assert yamlrocks.loads(doc.to_yaml()) == {"x": [1, 2, "three"]}


def test_assign_dict_emits_nested_mapping():
    """Assigning a dict emits a nested mapping under the key."""
    doc = yamlrocks.loads(b"x: 0\n", option=RT)
    doc["x"] = {"a": 1, "b": 2}
    assert yamlrocks.loads(doc.to_yaml()) == {"x": {"a": 1, "b": 2}}


def test_assign_unsupported_type_raises_type_error():
    """Assigning an unconvertible Python object raises a TypeError."""
    doc = yamlrocks.loads(b"x: 0\n", option=RT)
    with pytest.raises(TypeError, match="cannot convert"):
        doc["x"] = object()


def test_assign_cyclic_structure_raises_not_crashes():
    """Assigning a self-referential object raises instead of crashing."""
    doc = yamlrocks.loads(b"x: 1\n", option=RT)
    d: dict = {}
    d["self"] = d
    with pytest.raises(ValueError, match="deeply nested"):
        doc["x"] = d
