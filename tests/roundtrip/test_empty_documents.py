"""Empty documents in a stream survive a re-emit of an edited document."""

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP


def edit_first_mapping(doc):
    """Change the first key of the first mapping document to a new value."""
    roots = doc.to_dict()
    if isinstance(roots, dict):
        key = next(iter(roots))
        doc[key] = "edited"
        return

    # A stream with several content documents edits one by assigning it whole.
    index = next(i for i, root in enumerate(roots) if isinstance(root, dict))
    key = next(iter(roots[index]))
    doc[index] = {**roots[index], key: "edited"}


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        (b"---\n---\na: 1\n", b"---\n---\na: edited\n"),
        (b"a: 1\n---\n---\nb: 2\n", b"a: edited\n---\n---\nb: 2\n"),
        (b"a: 1\n---\n", b"a: edited\n---\n"),
        (b"a: 1\n--- # note\n", b"a: edited\n--- # note\n"),
        (
            b"---\n# only a comment\n---\na: 1\n",
            b"---\n# only a comment\n---\na: edited\n",
        ),
        (
            b"a: 1\n---\n# one\n---\n# two\n---\nb: 2\n",
            b"a: edited\n---\n# one\n---\n# two\n---\nb: 2\n",
        ),
        (b"a: 1\n---\n# trailing document\n", b"a: edited\n---\n# trailing document\n"),
        (b"---\n...\na: 1\n", b"---\n...\na: edited\n"),
        (b"---\n...\n---\na: 1\n", b"---\n...\n---\na: edited\n"),
        (b"%YAML 1.2\n---\n---\na: 1\n", b"%YAML 1.2\n---\n---\na: edited\n"),
        (
            b"a: 1\n...\n%YAML 1.2\n---\n# Empty\n...\n",
            b"a: edited\n...\n%YAML 1.2\n---\n# Empty\n...\n",
        ),
        (
            "﻿# head\n---\n---\na: 1\n".encode(),
            "﻿# head\n---\n---\na: edited\n".encode(),
        ),
    ],
)
def test_edit_keeps_empty_documents(source, expected):
    """Editing a neighbouring document re-emits every empty document verbatim."""
    doc = yamlrocks.loads(source, option=RT)
    edit_first_mapping(doc)

    output = doc.to_yaml()

    assert output == expected
    assert len(yamlrocks.loads_all(output)) == len(yamlrocks.loads_all(source))


def test_ended_document_keeps_the_empty_one_after_it():
    """An empty document after a `...` keeps the stream's count and data."""
    doc = yamlrocks.loads(b"a: 1\n...\n---\n", option=RT)
    doc["a"] = 2

    assert yamlrocks.loads_all(doc.to_yaml()) == [{"a": 2}, None]


def test_empty_document_comment_stays_out_of_a_block_scalar():
    """The comment of an empty document never becomes block scalar content."""
    source = b"%YAML 1.2\n--- |\n%!PS-Adobe-2.0\n...\n%YAML 1.2\n---\n# Empty\n...\n---\nk: v\n"
    doc = yamlrocks.loads(source, option=RT)
    doc[1] = {"k": "w"}

    output = doc.to_yaml()

    assert yamlrocks.loads_all(output) == ["%!PS-Adobe-2.0\n", None, {"k": "w"}]
    assert b"\n...\n%YAML 1.2\n---\n# Empty\n...\n" in output


def test_upgrade_keeps_empty_documents():
    """Upgrading a stream rewrites its documents without dropping empty ones."""
    output = yamlrocks.upgrade(b"a: yes\n---\n# only a comment\n---\nb: no\n")

    assert yamlrocks.loads_all(output) == [{"a": True}, None, {"b": False}]
    assert b"---\n# only a comment\n---\n" in output


def test_replacing_a_root_keeps_its_empty_documents():
    """Assigning a whole document keeps the empty documents next to it."""
    doc = yamlrocks.loads(b"a: 1\n---\n---\nb: 2\n--- # end\n", option=RT)
    doc[1] = {"c": 3}

    assert doc.to_yaml() == b"a: 1\n---\n---\nc: 3\n--- # end\n"


def test_deleting_a_document_keeps_the_empty_ones_around_it():
    """Deleting a document removes only that document from the stream."""
    doc = yamlrocks.loads(b"a: 1\n---\n# x\n---\nb: 2\n--- # y\n", option=RT)
    del doc[1]

    output = doc.to_yaml()

    assert output == b"a: 1\n---\n# x\n--- # y\n"
    assert yamlrocks.loads_all(output) == [{"a": 1}, None, None]


@pytest.mark.parametrize(
    "source",
    [
        b"---\n---\na: 1\n",
        b"a: 1\n--- # note\n",
        b"---\n# only a comment\n---\na: 1\n",
    ],
)
def test_single_content_document_keeps_its_api(source):
    """A stream with one content document still behaves as that one document."""
    doc = yamlrocks.loads(source, option=RT)

    assert doc.to_dict() == {"a": 1}
    assert doc["a"] == 1
    assert len(doc) == 1
    assert doc.node.comment_before is None
    assert doc.node.comment_after is None


def test_included_file_keeps_its_empty_documents_to_itself(tmp_path):
    """An included file's empty documents never become documents of the includer."""
    (tmp_path / "part.yaml").write_text("---\n# part note\n---\na: 1\n")
    doc = yamlrocks.loads(
        b"!include part.yaml\n",
        option=RT | yamlrocks.OPT_INCLUDES,
        include_dir=str(tmp_path),
    )
    doc["a"] = 2

    output = doc.to_yaml()

    assert b"part note" not in output
    assert len(yamlrocks.loads_all(output, option=yamlrocks.OPT_PASSTHROUGH_TAG)) == 1
