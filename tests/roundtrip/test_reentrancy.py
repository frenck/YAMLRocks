"""Python code that touches a round-trip document while one of its own calls runs.

Converting a key or value can run arbitrary Python (``__eq__``, ``__index__``,
``__float__``, callbacks), and that code may read or change the very document
being edited. Every such sequence either works or raises an ordinary exception;
none may escape as a ``PanicException``, which ``except Exception`` cannot catch.
"""

import datetime
import threading

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP
SOURCE = b"a: [1]\nb: {x: 1}\n"


def load(source=SOURCE, option=RT):
    """Load `source` as a round-trip document."""
    return yamlrocks.loads(source, option=option)


def reread(doc):
    """Re-emit `doc` and read it back as plain data."""
    return yamlrocks.loads(doc.to_yaml())


def touch(doc):
    """Read `doc` through the accessors that borrow it."""
    doc.get("b", None)
    repr(doc.node)


class ReadsDocument:
    """A float whose conversion reads the document through its views."""

    def __init__(self, doc, result=1.5):
        self.doc = doc
        self.result = result

    def __float__(self):
        touch(self.doc)
        return float(self.result)


class ReadsDocumentAsIndex:
    """An integer whose conversion reads the document through its views."""

    def __init__(self, doc, result):
        self.doc = doc
        self.result = result

    def __index__(self):
        touch(self.doc)
        return self.result


class CountingKey:
    """A key that equals `name`, reads the document, and counts its calls."""

    def __init__(self, doc, name):
        self.doc = doc
        self.name = name
        self.eq_calls = 0
        self.hash_calls = 0

    def __eq__(self, other):
        self.eq_calls += 1
        touch(self.doc)
        return other == self.name

    def __hash__(self):
        self.hash_calls += 1
        touch(self.doc)
        return hash(self.name)


class CountingIndex:
    """A sequence index that reads the document and counts its conversions."""

    def __init__(self, doc, index):
        self.doc = doc
        self.index = index
        self.calls = 0

    def __index__(self):
        self.calls += 1
        touch(self.doc)
        return self.index


# -- Assigning views, cursors and documents ------------------------------------


def test_assigning_a_view_of_the_same_document_copies_it():
    """Assigning a view of the document into it stores a copy of its data."""
    doc = load()
    doc["c"] = doc["b"]
    assert reread(doc) == {"a": [1], "b": {"x": 1}, "c": {"x": 1}}


def test_assigning_the_root_cursor_copies_the_document_as_it_was():
    """Assigning the root cursor stores the document's data from before the write."""
    doc = load()
    doc["c"] = doc.node
    assert reread(doc) == {"a": [1], "b": {"x": 1}, "c": {"a": [1], "b": {"x": 1}}}


def test_setting_a_cursor_value_to_a_view_copies_it():
    """`node.value = view` replaces the node with a copy of the view's data."""
    doc = load()
    doc.node["a"].value = doc["b"]
    assert reread(doc) == {"a": {"x": 1}, "b": {"x": 1}}


def test_a_view_nested_in_a_list_is_copied():
    """A view inside an assigned list is copied like a top-level one."""
    doc = load()
    doc["c"] = [doc["b"]]
    assert reread(doc) == {"a": [1], "b": {"x": 1}, "c": [{"x": 1}]}


def test_one_document_of_a_stream_can_be_assigned_into_another():
    """A view of one document of a stream can be assigned into another."""
    stream = load(b"a: 0\n---\nb: [1]\n")
    stream[1]["copy"] = stream[0]
    assert yamlrocks.loads_all(stream.to_yaml()) == [
        {"a": 0},
        {"b": [1], "copy": {"a": 0}},
    ]


def test_assigning_a_view_into_its_own_child_does_not_loop():
    """A view assigned below itself is copied once, not nested forever."""
    doc = load()
    doc["b"]["self"] = doc["b"]
    assert reread(doc)["b"] == {"x": 1, "self": {"x": 1}}


def test_assigning_a_view_of_another_document_copies_it():
    """A view of an unrelated document is copied into the target."""
    source = load(b"settings: {debug: true, level: 3}\n")
    target = load(b"app: {}\n")
    target["app"]["settings"] = source["settings"]
    assert reread(target) == {"app": {"settings": {"debug": True, "level": 3}}}


def test_assigning_a_scalar_cursor_copies_its_value():
    """A cursor on a scalar copies as that scalar."""
    doc = load(b"a: 'quoted'\nb: 0\n")
    doc["b"] = doc.node["a"]
    assert reread(doc) == {"a": "quoted", "b": "quoted"}


def test_assigning_a_whole_stream_copies_its_documents_as_a_list():
    """A multi-document object as a value copies as the list of its documents."""
    stream = load(b"a: 1\n---\n- 2\n")
    target = load(b"x: 0\n")
    target["x"] = stream
    assert reread(target) == {"x": [{"a": 1}, [2]]}


def test_a_copy_drops_comments_and_anchors_and_expands_aliases():
    """A copy carries data only: comments and anchors stay behind, aliases expand."""
    doc = load(b"base: &b {v: 1}  # shared\nref: *b\nsrc:\n  # note\n  k: *b\n")
    doc["dst"] = doc["src"]
    out = doc.to_yaml()
    assert out.count(b"&b") == 1
    assert out.count(b"# note") == 1
    assert yamlrocks.loads(out)["dst"] == {"k": {"v": 1}}


def test_a_copy_keeps_a_merge_key_a_merge_key():
    """A copied `<<` still merges in the target."""
    doc = load(b"base: &b {v: 1}\nsrc:\n  <<: *b\n  w: 2\n")
    doc["dst"] = doc["src"]
    assert reread(doc)["dst"] == {"v": 1, "w": 2}


def test_a_copy_is_quoted_for_the_target_schema():
    """A string copied into a YAML 1.1 document is quoted where 1.1 would retype it."""
    source = load(b"flag: yes\n")
    target = load(b"x: 0\n", option=RT | yamlrocks.OPT_YAML_1_1)
    target["x"] = source.node["flag"]
    assert yamlrocks.loads(target.to_yaml(), option=yamlrocks.OPT_YAML_1_1) == {
        "x": "yes"
    }


def test_a_copy_round_trips_a_collection_key():
    """A mapping key that is a sequence survives being copied through a view."""
    doc = load(b"src:\n  ? [a, b]\n  : 1\n")
    doc["dst"] = doc["src"]
    assert reread(doc)["dst"] == {("a", "b"): 1}
    assert load(doc.to_yaml()).to_dict()["dst"] == {("a", "b"): 1}


def test_a_copy_round_trips_a_timestamp():
    """A date read under OPT_TIMESTAMPS survives being copied through a view."""
    option = RT | yamlrocks.OPT_TIMESTAMPS
    doc = load(b"src: {when: 2024-01-02, at: 2024-01-02 03:04:05+01:00}\n", option)
    doc["dst"] = doc["src"]
    copied = load(doc.to_yaml(), option).to_dict()["dst"]
    assert copied["when"] == datetime.date(2024, 1, 2)
    assert copied["at"] == datetime.datetime(
        2024, 1, 2, 3, 4, 5, tzinfo=datetime.timezone(datetime.timedelta(hours=1))
    )


def test_assigning_a_stale_view_raises_key_error():
    """A view whose node is gone cannot be copied."""
    doc = load()
    view = doc["b"]
    del doc["b"]
    with pytest.raises(KeyError, match="stale view"):
        doc["c"] = view
    assert reread(doc) == {"a": [1]}


# -- Conversions that read the document ----------------------------------------


def test_a_value_whose_float_reads_the_document_is_assigned():
    """A value's `__float__` may read the document it is assigned into."""
    doc = load()
    doc["c"] = ReadsDocument(doc)
    assert reread(doc)["c"] == 1.5


def test_a_value_whose_index_reads_the_document_is_assigned_into_a_list():
    """A value's `__index__` may read the document during a sequence write."""
    doc = load()
    doc["a"][0] = ReadsDocumentAsIndex(doc, 7)
    assert reread(doc)["a"] == [7]


def test_a_cursor_value_whose_float_reads_the_document_is_set():
    """A value's `__float__` may read the document while `node.value` is set."""
    doc = load()
    doc.node["b"]["x"].value = ReadsDocument(doc, result=2.5)
    assert reread(doc)["b"] == {"x": 2.5}


def test_a_document_of_a_stream_is_replaced_by_a_value_that_reads_it():
    """Replacing a whole document works when the value reads the stream."""
    stream = load(b"a: 0\n---\nb: {x: 1}\n")

    class ReadsStream:
        def __float__(self):
            assert stream[1]["b"]["x"] == 1
            return 3.0

    stream[0] = ReadsStream()
    assert yamlrocks.loads_all(stream.to_yaml()) == [3.0, {"b": {"x": 1}}]


def test_a_key_whose_eq_reads_the_document_is_matched_on_write():
    """A key's `__eq__` may read the document while an entry is replaced."""
    doc = load()
    doc[CountingKey(doc, "b")] = 2
    assert reread(doc) == {"a": [1], "b": 2}


def test_a_key_whose_eq_reads_the_document_is_matched_on_read():
    """A key's `__eq__` may read the document while an entry is looked up."""
    doc = load()
    assert doc[CountingKey(doc, "b")].to_dict() == {"x": 1}
    assert doc["b"][CountingKey(doc, "x")] == 1


def test_a_key_whose_eq_reads_the_document_is_matched_on_delete():
    """A key's `__eq__` may read the document while an entry is deleted."""
    doc = load()
    del doc[CountingKey(doc, "b")]
    assert reread(doc) == {"a": [1]}


def test_an_index_whose_index_reads_the_document_deletes_an_item():
    """An index's `__index__` may read the document while an item is deleted."""
    doc = load(b"a: [1, 2]\nb: {x: 1}\n")
    del doc["a"][CountingIndex(doc, 0)]
    assert reread(doc)["a"] == [2]


def test_an_index_whose_index_reads_the_document_reads_an_item():
    """An index's `__index__` may read the document while an item is read."""
    doc = load(b"a: [1, 2]\nb: {x: 1}\n")
    assert doc["a"][CountingIndex(doc, 1)] == 2
    assert doc.node["a"][CountingIndex(doc, 1)].value == 2


def test_an_index_whose_index_reads_the_stream_selects_a_document():
    """An index's `__index__` may read the stream while a document is read or deleted."""
    stream = load(b"a: 0\n---\nb: {x: 1}\n")

    class StreamIndex:
        def __index__(self):
            assert stream[1]["b"]["x"] == 1
            return 0

    assert stream[StreamIndex()]["a"] == 0
    del stream[StreamIndex()]
    assert stream.to_dict() == {"b": {"x": 1}}


def test_dumps_and_to_json_of_the_document_inside_a_conversion():
    """`dumps` and `to_json` of the document work from inside its own assignment."""
    doc = load()
    seen = []

    class Dumps:
        def __float__(self):
            seen.append(yamlrocks.dumps(doc))
            seen.append(yamlrocks.to_json(doc))
            return 2.0

    doc["c"] = Dumps()
    assert seen == [SOURCE, b'{"a":[1],"b":{"x":1}}']
    assert reread(doc)["c"] == 2.0


def test_a_tag_handler_reading_the_document_inside_a_conversion():
    """A `tag_handler` may read the document while it is being assigned into."""
    doc = load()

    class Loads:
        def __float__(self):
            loaded = yamlrocks.loads(
                b"x: !t 1\n", tag_handler=lambda tag, value: doc["b"]["x"]
            )
            return float(loaded["x"])

    doc["c"] = Loads()
    assert reread(doc)["c"] == 1.0


def test_represent_and_default_callbacks_may_touch_the_document_being_dumped():
    """`represent=` and `default=` callbacks may read and edit a document being dumped."""
    doc = load()

    def represent(value):
        if value == {"x": 1}:
            doc["seen"] = doc["b"]["x"]
        return None

    assert yamlrocks.dumps(doc["b"], represent=represent) == b"x: 1\n"

    def default(value):
        doc["defaulted"] = True
        return doc["b"].to_dict()

    assert yamlrocks.dumps({"v": object()}, default=default) == b"v:\n  x: 1\n"
    assert reread(doc) == {"a": [1], "b": {"x": 1}, "seen": 1, "defaulted": True}


# -- Conversions that change the document --------------------------------------


def test_a_value_that_changes_the_document_raises_and_writes_nothing():
    """A write whose value's conversion changed the document raises; the write is not made."""
    doc = load()

    class Writes:
        def __float__(self):
            doc["other"] = 1
            return 1.0

    with pytest.raises(RuntimeError, match="changed while"):
        doc["c"] = Writes()
    assert reread(doc) == {"a": [1], "b": {"x": 1}, "other": 1}


def test_a_value_that_shifts_a_sequence_does_not_overwrite_the_wrong_item():
    """A sequence write whose value deleted an earlier item raises instead of hitting its neighbour."""
    doc = load(b"a: [1, 2, 3]\n")

    class Shifts:
        def __float__(self):
            del doc["a"][0]
            return 9.0

    with pytest.raises(RuntimeError, match="changed while"):
        doc["a"][1] = Shifts()
    assert reread(doc) == {"a": [2, 3]}


def test_replacing_a_document_after_an_earlier_one_was_deleted_writes_nothing():
    """`stream[1] = value` raises when the value's conversion deletes `stream[0]`."""
    stream = load(b"a: 0\n---\nb: 1\n---\nc: 2\n")

    class DeletesFirst:
        def __float__(self):
            del stream[0]
            return 5.0

    with pytest.raises(RuntimeError, match="changed while"):
        stream[1] = DeletesFirst()
    assert yamlrocks.loads_all(stream.to_yaml()) == [{"b": 1}, {"c": 2}]


def test_a_key_that_changes_the_document_on_delete_removes_nothing():
    """A delete whose key's `__eq__` changed the document raises and removes nothing."""
    doc = load()

    class Writes:
        def __eq__(self, other):
            doc["other"] = 1
            return other == "b"

        __hash__ = None

    with pytest.raises(RuntimeError, match="changed while"):
        del doc[Writes()]
    assert reread(doc) == {"a": [1], "b": {"x": 1}, "other": 1}


def test_get_raises_when_the_key_changed_the_document():
    """`get()` raises instead of returning the default when the lookup was disturbed."""
    doc = load()

    class Writes:
        def __eq__(self, other):
            doc["other"] = 1
            return False

        __hash__ = None

    with pytest.raises(RuntimeError, match="changed while"):
        doc.get(Writes(), "default")
    assert doc.get("missing", "default") == "default"


def test_a_cursor_value_that_changes_the_document_writes_nothing():
    """`node.value = value` raises when the value's conversion changed the document."""
    doc = load()
    cursor = doc.node["a"]

    class Writes:
        def __float__(self):
            doc["other"] = 1
            return 1.0

    with pytest.raises(RuntimeError, match="changed while"):
        cursor.value = Writes()
    assert reread(doc) == {"a": [1], "b": {"x": 1}, "other": 1}


# -- Each Python object is converted once --------------------------------------


def test_a_sequence_index_is_converted_once():
    """A sequence write calls the index's `__index__` exactly once."""
    doc = load()
    index = CountingIndex(doc, 0)
    doc["a"][index] = 5
    assert index.calls == 1
    assert reread(doc)["a"] == [5]


def test_a_mapping_key_is_compared_once_per_key_and_never_hashed():
    """Matching a key against a one-key mapping calls `__eq__` once and `__hash__` never."""
    doc = load()
    key = CountingKey(doc, "x")
    doc["b"][key] = 2
    assert (key.eq_calls, key.hash_calls) == (1, 0)

    key = CountingKey(doc, "x")
    assert doc["b"][key] == 2
    assert (key.eq_calls, key.hash_calls) == (1, 0)

    key = CountingKey(doc, "x")
    del doc["b"][key]
    assert (key.eq_calls, key.hash_calls) == (1, 0)


def test_a_new_mapping_key_is_converted_once():
    """A key added to a mapping runs `__eq__` per existing key and `__index__` once."""
    doc = load(b"m: {k: 1}\nb: {x: 1}\n")

    class IntKey(CountingIndex):
        eq_calls = 0

        def __eq__(self, other):
            type(self).eq_calls += 1
            return False

        __hash__ = None

    key = IntKey(doc, 7)
    doc["m"][key] = "seven"
    assert (key.calls, IntKey.eq_calls) == (1, 1)
    assert reread(doc)["m"] == {"k": 1, 7: "seven"}


# -- No panic escapes ----------------------------------------------------------


def _reentrant_cases():
    """Every reentrant sequence above, each as a call that may raise anything."""
    doc = load()

    class Writes:
        def __float__(self):
            doc["w"] = 1
            return 1.0

        def __index__(self):
            doc["w"] = 1
            return 0

        def __eq__(self, other):
            doc["w"] = 1
            return False

        __hash__ = None

    def assign(target, key, value):
        target[key] = value

    def set_value(cursor, value):
        cursor.value = value

    stream = load(b"a: 0\n---\nb: [1]\n---\nc: 2\n")
    return [
        lambda: assign(doc, "c", doc["b"]),
        lambda: assign(doc, "c", doc.node),
        lambda: set_value(doc.node["a"], doc["b"]),
        lambda: assign(doc, "c", [doc["b"]]),
        lambda: assign(stream[1], "copy", stream[0]),
        lambda: assign(doc, "c", ReadsDocument(doc)),
        lambda: assign(doc["a"], 0, ReadsDocumentAsIndex(doc, 3)),
        lambda: assign(doc, CountingKey(doc, "b"), 1),
        lambda: doc[CountingKey(doc, "a")],
        lambda: doc.__delitem__(CountingKey(doc, "c")),
        lambda: doc["a"].__delitem__(CountingIndex(doc, 0)),
        lambda: assign(doc, "c", Writes()),
        lambda: assign(doc["a"], Writes(), 1),
        lambda: assign(doc, Writes(), 1),
        lambda: doc[Writes()],
        lambda: doc.get(Writes()),
        lambda: doc.__delitem__(Writes()),
        lambda: set_value(doc.node["b"], Writes()),
        lambda: assign(stream, 1, Writes()),
        lambda: stream[Writes()],
        lambda: stream.__delitem__(Writes()),
        lambda: yamlrocks.dumps(doc["b"], represent=lambda value: assign(doc, "r", 1)),
        lambda: yamlrocks.dumps(
            {"v": object()}, default=lambda value: assign(doc, "d", doc["b"])
        ),
        lambda: repr(doc["b"]),
        lambda: repr(doc.node),
    ]


def test_no_reentrant_call_escapes_as_a_base_exception():
    """Every reentrant sequence either works or raises an ordinary `Exception`."""
    for case in _reentrant_cases():
        try:
            case()
        except Exception:
            pass


def test_threads_sharing_one_document_never_panic():
    """Threads reading and writing one document raise only ordinary exceptions."""
    doc = load(b"a: [1, 2, 3]\nb: {x: 1}\nc: text\n")
    escaped: list[BaseException] = []
    stop = threading.Event()

    def work(worker):
        operations = [
            lambda: doc["b"]["x"],
            lambda: doc.__setitem__("c", worker),
            lambda: doc["a"].__setitem__(0, doc["b"]),
            lambda: doc.to_dict(),
            lambda: doc.node["b"]["x"].value,
            lambda: doc.to_yaml(),
            lambda: len(doc["a"]),
        ]
        turn = 0
        while not stop.is_set():
            try:
                operations[(worker + turn) % len(operations)]()
            except Exception:
                pass
            except BaseException as error:
                escaped.append(error)
                return
            turn += 1

    threads = [threading.Thread(target=work, args=(worker,)) for worker in range(8)]
    for thread in threads:
        thread.start()
    stop.wait(0.5)
    stop.set()
    for thread in threads:
        thread.join()
    assert escaped == []
    assert set(reread(doc)) == {"a", "b", "c"}
