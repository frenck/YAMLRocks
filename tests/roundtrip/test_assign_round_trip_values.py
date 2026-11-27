"""Assigning views, cursors, documents, and reentrant values never panics."""

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP

SINGLE = b"a: 1  # keep\nb:\n  x: [1, 2]\nl: [3, 4]\n"
STREAM = b"a: 1\n---\nb: 2\n"


def load(source):
    """Load a round-trip document."""
    return yamlrocks.loads(source, option=RT)


class ReadsOnFloat:
    """A value whose float conversion reads the document it is assigned into."""

    def __init__(self, doc):
        """Remember the document to read."""
        self.doc = doc

    def __float__(self):
        """Read the document, then convert."""
        assert self.doc["a"] == 1
        return 1.5


class ReadsOnIndex:
    """A value or key whose integer conversion reads the document."""

    def __init__(self, doc):
        """Remember the document to read."""
        self.doc = doc

    def __index__(self):
        """Read the document, then convert."""
        assert self.doc["a"] == 1
        return 1


class WritesOnFloat:
    """A value whose float conversion writes to the document."""

    def __init__(self, doc):
        """Remember the document to write."""
        self.doc = doc

    def __float__(self):
        """Write to the document, then convert."""
        self.doc["a"] = 9
        return 2.5


class ReadsOnItems(dict):
    """A dict subclass whose items() reads the document."""

    def __init__(self, doc):
        """Hold one entry and remember the document."""
        super().__init__(q=1)
        self.doc = doc

    def items(self):
        """Read the document, then list the entries."""
        assert self.doc["a"] == 1
        return super().items()


SINGLE_CASES = {
    "view of the same document": (
        lambda d: d.__setitem__("c", d["b"]),
        b"c:\n  x:\n  - 1\n  - 2\n",
    ),
    "the document itself": (
        lambda d: d.__setitem__("c", d),
        b"c:\n  a: 1\n  b:\n    x:\n    - 1\n    - 2\n  l:\n  - 3\n  - 4\n",
    ),
    "a node cursor": (
        lambda d: d.__setitem__("c", d.node["b"]),
        b"c:\n  x:\n  - 1\n  - 2\n",
    ),
    "a scalar node cursor": (lambda d: d.__setitem__("c", d.node["a"]), b"c: 1\n"),
    "a list holding views": (
        lambda d: d.__setitem__("c", [d["b"], {"k": d["l"]}]),
        b"c:\n- x:\n  - 1\n  - 2\n- k:\n  - 3\n  - 4\n",
    ),
    "a float that reads the document": (
        lambda d: d.__setitem__("c", ReadsOnFloat(d)),
        b"c: 1.5\n",
    ),
    "an index that reads the document": (
        lambda d: d.__setitem__("c", ReadsOnIndex(d)),
        b"c: 1\n",
    ),
    "a dict whose items() reads the document": (
        lambda d: d.__setitem__("c", ReadsOnItems(d)),
        b"c:\n  q: 1\n",
    ),
}


@pytest.mark.parametrize("case", SINGLE_CASES, ids=list(SINGLE_CASES))
def test_document_setitem_copies_the_current_value(case):
    """Assigning a round-trip object or a reentrant value adds a plain copy of its data."""
    edit, appended = SINGLE_CASES[case]
    doc = load(SINGLE)
    edit(doc)

    assert doc.to_yaml() == SINGLE + appended


def test_view_setitem_accepts_a_view_of_the_same_document():
    """A view write may take a view of its own document as the value."""
    doc = load(SINGLE)
    doc["l"][0] = doc["b"]

    assert doc["l"].to_dict() == [{"x": [1, 2]}, 4]


def test_cursor_value_accepts_a_view_of_the_same_document():
    """The node cursor's value setter may take a view of its own document."""
    doc = load(SINGLE)
    doc.node["a"].value = doc["b"]

    assert doc.to_dict()["a"] == {"x": [1, 2]}
    assert doc.to_dict()["b"] == {"x": [1, 2]}


def test_a_view_assigned_into_its_own_descendant_is_a_snapshot():
    """Assigning a view into itself stores its data as it was, once."""
    doc = load(SINGLE)
    doc["b"]["self"] = doc["b"]

    assert doc["b"].to_dict() == {"x": [1, 2], "self": {"x": [1, 2]}}


def test_a_view_assigned_over_its_own_ancestor():
    """Assigning a child view over its parent keeps the child's data."""
    doc = load(SINGLE)
    doc["b"] = doc["b"]["x"]

    assert doc.to_dict()["b"] == [1, 2]


def test_the_copy_does_not_follow_later_edits():
    """A copied view is independent of its source after the assignment."""
    doc = load(SINGLE)
    doc["c"] = doc["b"]
    doc["b"]["x"][0] = 7

    assert doc["c"].to_dict() == {"x": [1, 2]}


def test_a_value_may_write_to_the_document_while_converting():
    """A value whose conversion writes to the document lands after that write."""
    doc = load(SINGLE)
    doc["c"] = WritesOnFloat(doc)

    assert doc.to_dict()["a"] == 9
    assert doc.to_dict()["c"] == 2.5


def test_an_index_key_that_reads_the_document():
    """A sequence index whose conversion reads the document works."""
    doc = load(SINGLE)
    doc["l"][ReadsOnIndex(doc)] = 7

    assert doc["l"].to_dict() == [3, 7]


def test_nested_writes_with_reentrant_values():
    """A view write and a cursor write take a float that reads the document."""
    doc = load(SINGLE)
    doc["b"]["x"] = ReadsOnFloat(doc)
    doc.node["a"].value = ReadsOnFloat(doc)

    assert doc.to_dict()["a"] == 1.5
    assert doc.to_dict()["b"] == {"x": 1.5}


@pytest.mark.parametrize(
    ("edit", "expected"),
    [
        (lambda m: m.__setitem__(1, m[0]), b"a: 1\n---\na: 1\n"),
        (lambda m: m[1].__setitem__("copy", m[0]), b"a: 1\n---\nb: 2\ncopy:\n  a: 1\n"),
        (lambda m: m[0].__setitem__("c", m), b"a: 1\nc:\n- a: 1\n- b: 2\n---\nb: 2\n"),
        (
            lambda m: setattr(m[1].node["b"], "value", m[0].node),
            b"a: 1\n---\nb:\n  a: 1\n",
        ),
        (lambda m: m.__setitem__(1, ReadsOnFloat(m[0])), b"a: 1\n---\n1.5\n"),
    ],
)
def test_stream_writes_take_views_of_other_documents(edit, expected):
    """Writes into a stream accept views of its other documents and of the stream itself."""
    doc = load(STREAM)
    edit(doc)

    assert doc.to_yaml() == expected


def test_a_view_of_another_object_is_copied():
    """A view of a different round-trip object assigns as its plain data."""
    doc = load(SINGLE)
    doc["c"] = load(b"z: [9]\n")["z"]

    assert doc.to_yaml() == SINGLE + b"c:\n- 9\n"


def test_a_stale_view_as_a_value_raises_key_error():
    """Assigning a view whose document was deleted raises KeyError and changes nothing."""
    doc = load(STREAM)
    stale = doc[1]
    del doc[0]

    with pytest.raises(KeyError, match="stale view"):
        doc["c"] = stale
    assert doc.to_yaml() == b"---\nb: 2\n"


def test_every_listed_case_is_survivable_with_except_exception():
    """No listed assignment raises anything that `except Exception` does not catch."""
    edits = [edit for edit, _ in SINGLE_CASES.values()]
    edits += [
        lambda d: d["l"].__setitem__(0, d["b"]),
        lambda d: setattr(d.node["a"], "value", d["b"]),
        lambda d: d["b"].__setitem__("self", d["b"]),
        lambda d: d.__setitem__("b", d["b"]["x"]),
        lambda d: d.__setitem__("c", WritesOnFloat(d)),
        lambda d: d["l"].__setitem__(ReadsOnIndex(d), 7),
    ]
    stream_edits = [
        lambda m: m.__setitem__(1, m[0]),
        lambda m: m[1].__setitem__("copy", m[0]),
        lambda m: m[0].__setitem__("c", m),
    ]

    # A `PanicException` derives from `BaseException`, so it would escape this
    # handler and fail the test; anything ordinary is caught.
    survived = 0
    for source, batch in ((SINGLE, edits), (STREAM, stream_edits)):
        for edit in batch:
            doc = load(source)
            try:
                edit(doc)
            except Exception:
                pass
            survived += 1

    assert survived == len(edits) + len(stream_edits)
