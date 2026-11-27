"""Views and node cursors into one document of a multi-document stream."""

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP

STREAM = (
    b"%YAML 1.2\n"
    b"---\n"
    b"# first document\n"
    b"name: app  # the name\n"
    b"tags: [a, b]\n"
    b"---\n"
    b"# second document\n"
    b"server:\n"
    b"  host: localhost  # dev only\n"
    b"  ports:\n"
    b"    - 80\n"
    b"    - 443\n"
    b"  tls:\n"
    b"    cert: /etc/cert.pem\n"
    b"defaults: &d 3\n"
    b"retries: *d\n"
    b"---\n"
    b"last: true\n"
    b"# end of stream\n"
)

# The same stream with the second document edited, and every other byte as written.
SECOND_EDITED = STREAM.replace(b"host: localhost", b"host: example.com")


def load(source=STREAM):
    """Load a stream as a single round-trip object."""
    return yamlrocks.loads(source, option=RT)


def alone(index, source=STREAM):
    """Load the document at `index` of a stream on its own, as the reference."""
    return yamlrocks.loads_all(source, option=RT)[index]


@pytest.mark.parametrize("index", [0, 1, 2])
def test_a_view_reads_like_the_document_loaded_alone(index):
    """Every read through a view of document N matches that document loaded alone."""
    view = load()[index]
    reference = alone(index)

    assert view.to_dict() == reference.to_dict()
    assert view.unwrap() == reference.to_dict()
    assert len(view) == len(reference)
    assert view.keys() == reference.keys()
    assert view.walk() == reference.walk()
    assert view.range() == reference.range()
    assert all(key in view for key in reference.to_dict())
    assert "missing" not in view
    assert view.get("missing", "fallback") == "fallback"
    assert repr(view) == "YAMLRocksDocumentView(mapping)"


def test_nested_views_read_two_and_three_levels_deep():
    """Nested views into a later document resolve inside that document."""
    doc = load()

    assert doc[1]["server"]["host"] == "localhost"
    assert doc[1]["server"]["ports"][1] == 443
    assert doc[1]["server"]["tls"]["cert"] == "/etc/cert.pem"
    assert doc[1]["server"]["tls"].to_dict() == {"cert": "/etc/cert.pem"}
    assert doc[1]["server"]["ports"].to_yaml() == alone(1)["server"]["ports"].to_yaml()


def test_an_alias_resolves_against_its_own_document():
    """An alias reads the anchor of its own document, not a same-named one before it."""
    doc = load(b"a: &x 1\nr: *x\n---\nb: &x 2\nr: *x\n")

    assert doc[0]["r"] == 1
    assert doc[1]["r"] == 2
    assert doc.to_dict() == [{"a": 1, "r": 1}, {"b": 2, "r": 2}]
    assert doc[1].node["r"].value == 2
    assert doc[1].node["r"].target.line == 4
    assert [node.line for node in doc[1].node["b"].aliases] == [5]
    assert [node.line for node in doc[0].node["a"].aliases] == [2]


def test_node_cursor_reads_metadata_of_a_later_document():
    """A node cursor into document N reports that document's comments and markup."""
    doc = load()
    reference = alone(1)

    cursor = doc[1].node
    assert cursor.comment_before == reference.node.comment_before == "second document"
    assert cursor.comment_after is None
    assert doc[2].node.comment_after == alone(2).node.comment_after == "end of stream"
    assert cursor.style == "block"
    assert cursor["server"]["host"].comment == "dev only"
    assert cursor["server"]["host"].line == reference.node["server"]["host"].line
    assert cursor["defaults"].anchor == "d"
    assert cursor["retries"].is_alias
    assert cursor["retries"].target.value == 3
    assert cursor["server"]["ports"][0].value == 80
    assert cursor["server"]["tls"]["cert"].tag is None
    assert "host" in cursor["server"]


def test_edit_through_a_view_keeps_the_other_documents_byte_for_byte():
    """Setting a value in document N re-emits every other document as written."""
    doc = load()
    doc[1]["server"]["host"] = "example.com"

    assert doc.to_yaml() == SECOND_EDITED


def test_each_edit_kind_reaches_only_its_document():
    """Set, delete, and cursor writes through document N leave the rest intact."""
    doc = load()
    doc[1]["server"]["ports"][0] = 8080
    del doc[1]["server"]["tls"]
    doc[1].node["server"]["host"].comment = "staging"
    doc[2]["last"] = False

    assert doc.to_yaml() == (
        STREAM.replace(b"- 80\n", b"- 8080\n")
        .replace(b"  tls:\n    cert: /etc/cert.pem\n", b"")
        .replace(b"localhost  # dev only", b"localhost # staging")
        .replace(b"last: true", b"last: false")
    )
    assert doc[0].to_dict() == alone(0).to_dict()


def test_node_cursor_writes_reach_a_later_document():
    """Comment, anchor, and value setters on a cursor into document N write there."""
    doc = load()
    cursor = doc[1].node
    cursor.comment_after = "the end"
    cursor["server"]["tls"]["cert"].value = "/etc/new.pem"
    cursor["server"]["host"].anchor = "h"

    out = doc.to_yaml()
    assert b"retries: *d\n# the end\n---\nlast: true\n" in out
    assert b"cert: /etc/new.pem" in out
    assert b"host: &h localhost" in out
    assert out.startswith(STREAM[: STREAM.index(b"# second document")])


def test_an_anchor_name_may_repeat_in_another_document():
    """Setting an anchor name used by another document is allowed; anchors are per document."""
    doc = load(b"a: &x 1\n---\nb: 2\n")
    doc[1].node["b"].anchor = "x"

    assert doc.to_yaml() == b"a: &x 1\n---\nb: &x 2\n"


def test_make_alias_and_detach_stay_inside_their_document():
    """make_alias and detach on a cursor into document N use that document's anchors."""
    doc = load(b"a: &x 1\n---\nb: &y 2\nc: 3\nr: *y\n")

    with pytest.raises(ValueError, match="no anchor '&x'"):
        doc[1].node["c"].make_alias("x")

    doc[1].node["c"].make_alias("y")
    assert doc[1].node["r"].detach().value == 2
    assert doc.to_yaml() == b"a: &x 1\n---\nb: &y 2\nc: *y\nr: 2\n"


def test_delete_guard_checks_the_aliases_of_its_own_document():
    """Deleting an anchor still used in its document is refused, whatever other documents hold."""
    doc = load(b"a: &x 1\n---\nb: &x 2\nr: *x\n")

    with pytest.raises(ValueError, match="would remove anchor '&x'"):
        del doc[1]["b"]

    assert doc.to_yaml() == b"a: &x 1\n---\nb: &x 2\nr: *x\n"


def test_dumps_of_a_stream_and_of_a_view():
    """dumps re-emits the whole stream and serializes a view as its plain value."""
    doc = load()
    doc[1]["server"]["host"] = "example.com"

    assert yamlrocks.dumps(doc) == SECOND_EDITED
    assert yamlrocks.loads(yamlrocks.dumps(doc[2])) == {"last": True}


def test_scalar_documents_read_and_replace():
    """A stream of scalar documents indexes to the scalars and replaces one in place."""
    doc = load(b"hello\n---\nworld\n")

    assert doc[1] == "world"
    doc[1] = "there"
    assert doc.to_yaml() == b"hello\n---\nthere\n"


def test_save_writes_an_edit_through_a_view(tmp_path):
    """save() writes back a stream edited through a view of a later document."""
    path = tmp_path / "stream.yaml"
    path.write_bytes(b"a: 1\n---\nb: 2  # keep\n")
    doc = yamlrocks.load(path, option=RT)
    doc[1]["b"] = 3

    assert doc.save() == [str(path)]
    assert path.read_bytes() == b"a: 1\n---\nb: 3  # keep\n"


def test_save_writes_an_include_edited_through_a_later_document(tmp_path):
    """An include in document N, edited through a view, is written back on its own."""
    (tmp_path / "part.yaml").write_bytes(b"x: 1  # keep\n")
    root = tmp_path / "main.yaml"
    root.write_bytes(b"a: 1\n---\nsub: !include part.yaml\n")
    doc = yamlrocks.load(root, option=RT | yamlrocks.OPT_INCLUDES)

    assert doc[1].node["sub"].is_include
    doc[1]["sub"]["x"] = 2

    assert doc.save() == [str(tmp_path / "part.yaml")]
    assert (tmp_path / "part.yaml").read_bytes() == b"x: 2  # keep\n"
    assert root.read_bytes() == b"a: 1\n---\nsub: !include part.yaml\n"


def test_document_wide_accessors_address_the_first_document():
    """node, locate, anchors, and walk on a stream address its first document."""
    doc = load(b"a: &x 1\n---\nb: &y 2\n")

    assert doc.node.value == {"a": 1}
    assert doc.locate(["a"]).value == 1
    assert doc.locate(["b"]) is None
    assert list(doc.anchors) == ["x"]
    assert doc.anchors["x"].value == 1
    assert doc.walk() == [(("a",), 1)]


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        (
            b"--- # lead\n---\na: 1\n--- # mid\n---\nb: 2\n--- # tail\n",
            b"--- # lead\n---\na: 1\n--- # mid\n---\nb: 3\n--- # tail\n",
        ),
        (b"a: 1\n---\n---\nb: 2\n", b"a: 1\n---\n---\nb: 3\n"),
        (
            b"a: 1\n---\n# only a comment\n---\nb: 2\n",
            b"a: 1\n---\n# only a comment\n---\nb: 3\n",
        ),
    ],
)
def test_empty_documents_stay_in_place_around_an_edit(source, expected):
    """A view index counts documents with content, and an edit keeps the empty ones where they are."""
    doc = load(source)

    assert len(doc) == 2
    assert doc[1]["b"] == 2
    doc[1]["b"] = 3
    assert doc.to_yaml() == expected


def test_replacing_the_value_of_a_document_root_keeps_its_empty_documents():
    """Setting a document root's value through its cursor keeps the empty documents around it."""
    doc = load(b"a: 1\n--- # mid\n---\nb: 2\n--- # tail\n")
    doc[1].node.value = {"c": 3}

    assert doc.to_yaml() == b"a: 1\n--- # mid\n---\nc: 3\n--- # tail\n"


def test_directives_and_markers_survive_an_edit_of_another_document():
    """Editing a later document keeps an earlier document's directives and end marker."""
    source = (
        b"%YAML 1.2\n---\na: 1\n...\n"
        b"%TAG !e! tag:example.com,2000:\n---\nb: !e!x 2\n"
        b"---\n# c\nc: [1, 2]\n"
    )
    doc = load(source)
    doc[2]["c"][0] = 7

    assert doc.to_yaml() == source.replace(b"[1, 2]", b"[7, 2]")


def test_handles_go_stale_when_a_delete_leaves_one_document():
    """Views and cursors taken on a two-document stream go stale once one is deleted."""
    doc = load(b"a: 1\n---\nb: 2\n")
    first, second = doc[0], doc[1]
    second_cursor, root_cursor = doc[1].node["b"], doc.node

    del doc[0]

    for read in (lambda: first["a"], lambda: second["b"], lambda: len(second)):
        with pytest.raises(KeyError, match="stale view"):
            read()
    for read in (lambda: second_cursor.value, lambda: root_cursor.value):
        with pytest.raises(KeyError, match="stale node"):
            read()
    with pytest.raises(KeyError, match="stale view"):
        second["b"] = 5
    assert doc.to_yaml() == b"---\nb: 2\n"
    assert doc["b"] == 2


def test_a_stale_handle_does_not_read_inside_the_surviving_document():
    """A handle to document 1 never reads index 1 of the sequence document left behind."""
    doc = load(b"- x\n- y\n---\n- p\n- q\n")
    second = doc[1]
    del doc[0]

    with pytest.raises(KeyError, match="stale view"):
        second[1]
    assert doc[1] == "q"


@pytest.mark.parametrize("deleted", [0, 1, 2])
def test_handles_go_stale_when_any_document_is_deleted(deleted):
    """Every handle on a three-document stream goes stale once a document is deleted."""
    doc = load(b"a: 1\n---\nb: 2\n---\nc: 3\n")
    views = [doc[0], doc[1], doc[2]]
    del doc[deleted]

    for view in views:
        with pytest.raises(KeyError, match="stale view"):
            view.to_dict()
    assert len(doc) == 2


def test_a_view_follows_a_replaced_document():
    """A view of document N reads the new value after document N is assigned whole."""
    doc = load(b"a: 1\n---\nb: 2\n")
    second = doc[1]
    doc[1] = {"z": 9}

    assert second["z"] == 9
    assert "b" not in second


def test_a_single_document_keeps_its_unprefixed_paths():
    """A single-document object indexes straight into its root, as before."""
    doc = load(b"- a\n- b\n")

    assert doc[1] == "b"
    assert doc.node[1].value == "b"
    assert doc.locate([1]).value == "b"


MIXED_VERSIONS = [
    b"%YAML 1.2\n---\na: yes\n...\n%YAML 1.1\n---\nb: yes\n",
    b"%YAML 1.1\n---\na: yes\n...\n%YAML 1.2\n---\nb: yes\n",
    b"%YAML 1.1\n---\na: yes\n...\n---\nb: yes\n",
    b"a: yes\n---\nb: yes\n...\n%YAML 1.1\n---\nc: yes\n",
]


@pytest.mark.parametrize("source", MIXED_VERSIONS)
def test_reads_use_the_version_of_their_own_document(source):
    """Reads through document N resolve scalars under N's own `%YAML` version."""
    doc = load(source)
    reference = yamlrocks.loads_all(source, option=RT)

    assert doc.to_dict() == [each.to_dict() for each in reference]
    for index, each in enumerate(reference):
        key = each.keys()[0]
        assert doc[index][key] == each[key]
        assert doc[index].node[key].value == each.node[key].value
        assert doc[index].keys() == each.keys()


@pytest.mark.parametrize("source", MIXED_VERSIONS)
def test_writes_quote_by_the_version_of_their_own_document(source):
    """A write through document N quotes exactly as the per-document object does."""
    for index in range(len(load(source))):
        doc = load(source)
        reference = yamlrocks.loads_all(source, option=RT)
        key = reference[index].keys()[0]

        doc[index][key] = "y"
        reference[index][key] = "y"

        # The edited document's bytes equal the per-document object's. The
        # stream around it differs only by the `...` markers the stream keeps.
        out = doc.to_yaml()
        assert reference[index].to_yaml() in out
        assert yamlrocks.loads_all(out, option=RT)[index][key] == "y"
