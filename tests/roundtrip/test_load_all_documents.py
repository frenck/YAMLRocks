"""Round-trip objects for individual documents in a YAML stream."""

import io

import pytest

import yamlrocks


@pytest.mark.parametrize(
    "source",
    [
        b"a: 1\n---\nb: 2\n",
        b"# header\n---\na: 'one' # inline\n...\n---\n# second\nb: [2, 3]\n",
        b"---\n---\nvalue\n---\n",
        b"%YAML 1.1\n---\na: yes\n...\n%YAML 1.2\n---\na: yes\n",
        "\ufeff---\nkey: café\n---\nother: naïve\n".encode(),
    ],
)
def test_per_document_roundtrip(source):
    """Per-document source slices reassemble the original byte-for-byte."""
    documents = yamlrocks.load_all(io.BytesIO(source), option=yamlrocks.OPT_ROUND_TRIP)
    assert all(isinstance(doc, yamlrocks.YAMLRocksDocument) for doc in documents)
    assert b"".join(doc.to_yaml() for doc in documents) == source


def test_empty_documents_are_not_lost():
    """Explicit empty documents stay distinct from an empty stream."""
    assert yamlrocks.loads_all(b"", option=yamlrocks.OPT_ROUND_TRIP) == []
    docs = yamlrocks.loads_all(b"---\n---\n42\n---\n", option=yamlrocks.OPT_ROUND_TRIP)
    assert [doc.to_dict() for doc in docs] == [None, 42, None]


def test_documents_are_independent():
    """Edits and same-named anchors are confined to their own document."""
    docs = yamlrocks.loads_all(
        b"a: &x 1\nb: *x\n---\na: &x 2\nb: *x\n", option=yamlrocks.OPT_ROUND_TRIP
    )
    assert [doc.to_dict() for doc in docs] == [{"a": 1, "b": 1}, {"a": 2, "b": 2}]
    docs[0]["a"] = 3
    assert docs[1].to_dict() == {"a": 2, "b": 2}


def test_each_document_uses_its_schema():
    """A version directive applies to its own document, not later documents."""
    docs = yamlrocks.loads_all(
        b"%YAML 1.1\n---\na: yes\n...\n---\na: yes\n", option=yamlrocks.OPT_ROUND_TRIP
    )
    assert [doc.to_dict() for doc in docs] == [{"a": True}, {"a": "yes"}]
