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


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        (b"---\n...\n  a: 1\n  b: 2\n", b"---\n...\na: edited\nb: 2\n"),
        (b"---\n...\n&x key: value\na: 1\n", b"---\n...\n&x key: edited\na: 1\n"),
        (b"---\n...\n!!map\na: 1\n", b"---\n...\n!!map\na: edited\n"),
        (b"---\n...\n# between\na: 1\n", b"---\n...\n# between\na: edited\n"),
    ],
)
def test_bare_root_after_a_closed_empty_document(source, expected):
    """A root without `---` after an empty `...` document keeps its first line."""
    doc = yamlrocks.loads(source, option=RT)
    edit_first_mapping(doc)

    output = doc.to_yaml()

    assert output == expected
    assert yamlrocks.loads_all(output) == [None, yamlrocks.loads_all(expected)[1]]


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        (b"---\n...\n  plain\n", b"---\n...\nplain # note\n"),
        (b"---\n...\n  [1, 2]\n", b"---\n...\n[1, 2] # note\n"),
        (b"---\n...\n  {a: 1}\n", b"---\n...\n{a: 1} # note\n"),
    ],
)
def test_bare_scalar_or_flow_root_after_a_closed_empty_document(source, expected):
    """A scalar or flow root after an empty `...` document is not duplicated."""
    doc = yamlrocks.loads(source, option=RT)
    doc.node.comment = "note"

    output = doc.to_yaml()

    assert output == expected
    assert yamlrocks.loads_all(output) == yamlrocks.loads_all(source)


def test_bare_block_scalar_root_after_a_closed_empty_document():
    """A block scalar root after an empty `...` document keeps its value."""
    doc = yamlrocks.loads(b"---\n...\n|\n  text\n", option=RT)
    doc.node.comment = "note"

    assert yamlrocks.loads_all(doc.to_yaml()) == [None, "text\n"]


def test_bare_root_after_a_closed_empty_document_with_crlf():
    """CRLF line breaks do not let the empty document swallow the root's line."""
    doc = yamlrocks.loads(b"---\r\n...\r\n  a: 1\r\n", option=RT)
    doc["a"] = 2

    output = doc.to_yaml()

    assert output.startswith(b"---\r\n...\r\na: 2")
    assert yamlrocks.loads_all(output) == [None, {"a": 2}]


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        (b"# head\n---\n---\na: yes\n", b"%YAML 1.2\n# head\n---\n---\na: true\n"),
        (
            b"# head\n%TAG !e! tag:x:\n---\n---\na: yes\n",
            b"%YAML 1.2\n# head\n%TAG !e! tag:x:\n---\n---\na: true\n",
        ),
        (
            b"%TAG !e! tag:x:\n# head\n---\n---\na: yes\n",
            b"%YAML 1.2\n%TAG !e! tag:x:\n# head\n---\n---\na: true\n",
        ),
        (
            b"# head\n%YAML 1.1\n---\n---\na: yes\n",
            b"%YAML 1.2\n# head\n---\n---\na: true\n",
        ),
        (
            b"# head\n%YAML 1.2\n---\n---\na: yes\n",
            b"%YAML 1.2\n# head\n---\n---\na: yes\n",
        ),
        (
            "\ufeff# head\n---\n---\na: yes\n".encode(),
            "\ufeff%YAML 1.2\n# head\n---\n---\na: true\n".encode(),
        ),
        (b"# head\n---\na: 1\n", b"%YAML 1.2\n# head\n---\na: 1\n"),
    ],
)
def test_upgrade_stamps_ahead_of_leading_comments(source, expected):
    """Upgrade puts `%YAML 1.2` ahead of leading comments without a new document."""
    output = yamlrocks.upgrade(source)

    assert output == expected
    assert len(yamlrocks.loads_all(output)) == len(yamlrocks.loads_all(source))


INCLUDES = RT | yamlrocks.OPT_INCLUDES | yamlrocks.OPT_SECRETS | yamlrocks.OPT_ENV_VAR


@pytest.fixture
def include_dir(tmp_path, monkeypatch):
    """A config directory with an included file, a secret, and an env var."""
    (tmp_path / "part.yaml").write_text("---\n# part note\n---\na: 1\n")
    (tmp_path / "secrets.yaml").write_text("password: hunter2\n")
    monkeypatch.setenv("YAMLROCKS_EMPTY_DOCUMENTS_TEST", "value")
    return tmp_path


@pytest.mark.parametrize(
    "source", [b"k: !include part.yaml\n", b"!include part.yaml\n"]
)
def test_include_write_back_keeps_the_files_empty_documents(include_dir, source):
    """Writing back an edited included file replays its own empty documents."""
    doc = yamlrocks.loads(source, option=INCLUDES, include_dir=str(include_dir))
    included = doc["k"] if source.startswith(b"k:") else doc
    included["a"] = 2

    written = yamlrocks.dump_includes_map(doc)[str(include_dir / "part.yaml")]

    assert written == b"---\n# part note\n---\na: 2\n"
    assert yamlrocks.loads_all(written) == [None, {"a": 2}]


def test_include_save_keeps_the_files_empty_documents(include_dir):
    """`save()` writes an edited included file with its empty documents intact."""
    root = include_dir / "root.yaml"
    root.write_text("k: !include part.yaml\n")
    doc = yamlrocks.load(str(root), option=INCLUDES, include_dir=str(include_dir))
    doc["k"]["a"] = 2

    doc.save()

    assert (include_dir / "part.yaml").read_bytes() == b"---\n# part note\n---\na: 2\n"


@pytest.mark.parametrize(
    ("source", "count", "markers"),
    [
        (b"!include part.yaml\n---\n", 2, []),
        (b"---\n---\n!include part.yaml\n---\n", 3, []),
        (b"!env_var YAMLROCKS_EMPTY_DOCUMENTS_TEST\n--- # end\n", 2, [b"--- # end\n"]),
        (b"--- # start\n---\n!secret password\n---\n", 3, [b"--- # start\n"]),
    ],
)
def test_resolved_root_keeps_its_streams_empty_documents(
    include_dir, source, count, markers
):
    """A root resolved from a tag keeps the empty documents of its own stream."""
    doc = yamlrocks.loads(source, option=INCLUDES, include_dir=str(include_dir))
    doc.node.comment = "edited"

    output = doc.to_yaml()

    documents = yamlrocks.loads_all(output, option=yamlrocks.OPT_PASSTHROUGH_TAG)
    assert len(documents) == count
    assert b"part note" not in output
    for marker in markers:
        assert marker in output
