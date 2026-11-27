"""Assigning a view, cursor, or document copies its data faithfully and within bounds.

The copy must read back, through the target document, as exactly the values
the source reads as (type included), must not let alias expansion run away, and
must never store a partial result.
"""

import datetime
import math
import subprocess
import sys
import time

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP
TIMESTAMPS = yamlrocks.OPT_TIMESTAMPS

OPTIONS = {
    "default": RT,
    "yaml-1.1": RT | yamlrocks.OPT_YAML_1_1,
    "pyyaml": RT | yamlrocks.OPT_PYYAML_COMPAT,
    "timestamps": RT | TIMESTAMPS,
    "yaml-1.1+timestamps": RT | yamlrocks.OPT_YAML_1_1 | TIMESTAMPS,
}

# Scalars whose reading depends on the schema, the timestamp setting, or their
# quoting, written as they appear after `key: `.
AWKWARD = [
    "yes", "no", "on", "off", "y", "n", "Yes", "NO",
    "null", "~", "true", "TRUE", "True", "false",
    "1_000", "0x1F", "0o17", "0b101", "017", "-0x1F", "+12",
    "12:30:00", "1:30", ".inf", "-.inf", ".nan", "1e3", "1.5e3",
    "9223372036854775808", "9_223_372_036_854_775_808", "+9223372036854775808",
    "-9223372036854775809", "0x8000_0000_0000_0000",
    "2024-01-02", "2024-01-02 03:04:05", "2024-01-02T03:04:05Z",
    "x,y", "a b", "-x", ":x", "0.0.0.0",
]  # fmt: skip
VALUES = (
    AWKWARD
    + [f"'{text}'" for text in AWKWARD]
    + [f'"{text}"' for text in AWKWARD]
    + ["' lead'", "'trail '", "'a: b'", "'a #b'", '"multi\\nline"', "''", "'it''s'"]
    + ["!!str 123", "!!int '12'", "!!float 1", "!!bool 'yes'", "!custom thing", ""]
    + ["|\n    line one\n    line two", ">\n    folded\n    text", "|-\n    kept"]
)

# Keys of one mapping that share their text but not their reading.
KEY_GROUPS = [
    ["TRUE", "'true'", "true", "True"],
    ["1", "'1'", "1.0", "0x1", "01"],
    ["~", "null", "'null'", "''"],
    ["yes", "'yes'", "on", "y"],
    ["2024-01-02", "'2024-01-02'"],
]
PROBES = [True, "true", 1, "1", 1.0, None, "", "null", "yes", "y", "2024-01-02"]


def source(values=VALUES):
    """A document holding `values` under `src` and the key groups under `groups`."""
    lines = ["src:"]
    lines += [f"  k{index}: {value}" for index, value in enumerate(values)]
    lines.append("groups:")
    for group in KEY_GROUPS:
        lines.append("  -")
        lines += [f"    {key}: {index}" for index, key in enumerate(group)]
    return ("\n".join(lines) + "\n").encode()


def typed(value):
    """`value` in a form that compares equal only for equal values of equal types."""
    if isinstance(value, dict):
        return ("dict", tuple((typed(key), typed(item)) for key, item in value.items()))
    if isinstance(value, (list, tuple)):
        return (type(value).__name__, tuple(typed(item) for item in value))
    if isinstance(value, float) and math.isnan(value):
        return ("float", "nan")
    return (type(value).__name__, value)


def reads_timestamps(option):
    """Whether a document loaded with `option` reads timestamps as dates."""
    return bool(option & (TIMESTAMPS | yamlrocks.OPT_PYYAML_COMPAT))


def lookup(view, key):
    """What `view[key]` gives, or the type of what it raises."""
    try:
        found = view[key]
    except Exception as error:
        return ("raises", type(error).__name__)
    return typed(found.to_dict() if hasattr(found, "to_dict") else found)


def alias_bomb(levels):
    """A document of `levels` anchors, each a list of nine aliases to the last."""
    lines = ["a0: &a0 [" + ", ".join(["x"] * 9) + "]"]
    for level in range(1, levels):
        lines.append(f"a{level}: &a{level} [" + ", ".join([f"*a{level - 1}"] * 9) + "]")
    return ("\n".join(lines) + "\n").encode()


# -- The copy reads back as its source -----------------------------------------


@pytest.mark.parametrize("option", OPTIONS.values(), ids=OPTIONS.keys())
def test_a_copy_within_one_document_reads_as_its_source(option):
    """Every awkward scalar and key copied within a document reads back with its value and type."""
    doc = yamlrocks.loads(source(), option=option)
    original = doc.to_dict()
    doc["copy_src"] = doc["src"]
    doc["copy_groups"] = doc["groups"]

    for reader in (doc, yamlrocks.loads(doc.to_yaml(), option=option)):
        copied = reader.to_dict()
        assert typed(copied["copy_src"]) == typed(original["src"])
        assert typed(copied["copy_groups"]) == typed(original["groups"])


@pytest.mark.parametrize("option", OPTIONS.values(), ids=OPTIONS.keys())
def test_lookups_in_a_copied_mapping_behave_as_in_its_source(option):
    """Keys that share text but not type are found in the copy exactly as in the source."""
    doc = yamlrocks.loads(source(), option=option)
    doc["copy_groups"] = doc["groups"]
    for index in range(len(KEY_GROUPS)):
        for key in PROBES:
            assert lookup(doc["copy_groups"][index], key) == lookup(
                doc["groups"][index], key
            )


@pytest.mark.parametrize(
    ("source_name", "target_name"),
    [
        ("default", "yaml-1.1"),
        ("yaml-1.1", "default"),
        ("default", "pyyaml"),
        ("pyyaml", "default"),
        ("timestamps", "default"),
        ("default", "timestamps"),
        ("yaml-1.1", "timestamps"),
    ],
)
def test_a_copy_into_another_document_reads_as_its_source(source_name, target_name):
    """A copy into a document read differently still reads as the source's values."""
    source_option, target_option = OPTIONS[source_name], OPTIONS[target_name]
    origin = yamlrocks.loads(source(), option=source_option)
    target = yamlrocks.loads(b"dst: 0\n", option=target_option)
    target["dst"] = origin["src"]

    want = origin.to_dict()["src"]
    got = yamlrocks.loads(target.to_yaml(), option=target_option).to_dict()["dst"]
    for key, value in want.items():
        if isinstance(value, datetime.date) and not reads_timestamps(target_option):
            # The target reads no dates, so its text is the only faithful form.
            assert isinstance(got[key], str)
        else:
            assert typed(got[key]) == typed(value), key


def test_a_big_integer_with_separators_stays_an_integer_across_schemas():
    """A YAML 1.1 big integer with `_` separators copies into YAML 1.2 as an integer."""
    origin = yamlrocks.loads(
        b"src: {big: 9_223_372_036_854_775_808, neg: -0x8000_0000_0000_0001}\n",
        option=RT | yamlrocks.OPT_YAML_1_1,
    )
    target = yamlrocks.loads(b"dst: 0\n", option=RT)
    target["dst"] = origin["src"]
    assert yamlrocks.loads(target.to_yaml()) == {
        "dst": {"big": 9223372036854775808, "neg": -9223372036854775809}
    }


def test_copied_keys_that_share_text_stay_distinct():
    """`TRUE` and `'true'` copy as two keys that look up as they do in the source."""
    doc = yamlrocks.loads(b"src: {TRUE: 1, 'true': 2}\n", option=RT)
    doc["dst"] = doc["src"]
    assert doc["dst"][True] == doc["src"][True]
    assert doc["dst"]["true"] == doc["src"]["true"]
    assert doc.to_dict()["dst"] == doc.to_dict()["src"] == {True: 1, "true": 2}


def test_a_date_shaped_string_stays_a_string_under_timestamps():
    """A quoted date copied, or assigned as a `str`, in a timestamp document stays a `str`."""
    option = RT | TIMESTAMPS
    doc = yamlrocks.loads(b"src: {when: '2024-01-02'}\n", option=option)
    doc["copy"] = doc["src"]
    doc["plain"] = {"when": "2024-01-02 03:04:05"}
    reread = yamlrocks.loads(doc.to_yaml(), option=option).to_dict()
    assert reread["copy"] == {"when": "2024-01-02"}
    assert reread["plain"] == {"when": "2024-01-02 03:04:05"}


def test_a_date_copied_where_timestamps_are_not_read_becomes_its_text():
    """A date copied into a document without OPT_TIMESTAMPS is stored as its own text."""
    origin = yamlrocks.loads(b"src: {when: 2024-01-02}\n", option=RT | TIMESTAMPS)
    target = yamlrocks.loads(b"dst: 0\n", option=RT)
    target["dst"] = origin["src"]
    assert yamlrocks.loads(target.to_yaml()) == {"dst": {"when": "2024-01-02"}}


def test_a_block_scalar_copies_as_its_text():
    """A literal block scalar copied at another indentation keeps its text."""
    doc = yamlrocks.loads(b"src:\n  text: |\n    one\n    two\n", option=RT)
    doc["deeper"] = {"inner": doc["src"]}
    assert yamlrocks.loads(doc.to_yaml())["deeper"] == {"inner": {"text": "one\ntwo\n"}}


# -- Bounded alias expansion ----------------------------------------------------


def test_an_alias_bomb_raises_quickly_and_writes_nothing():
    """Copying an eight-level alias bomb raises `ValueError` and leaves the document as it was."""
    doc = yamlrocks.loads(alias_bomb(8), option=RT)
    before = doc.to_yaml()
    view = doc["a7"]

    start = time.perf_counter()
    with pytest.raises(ValueError, match="too many nodes"):
        doc["copy"] = view
    assert time.perf_counter() - start < 10
    assert doc.to_yaml() == before
    assert yamlrocks.loads(doc.to_yaml(), option=RT).to_yaml() == before


def test_a_wide_anchor_referenced_many_times_raises():
    """A 512-item anchor referenced 2,000 times is over budget and is not copied."""
    base = "[" + ", ".join(str(item) for item in range(512)) + "]"
    doc = yamlrocks.loads(
        f"base: &b {base}\nrefs: [{', '.join(['*b'] * 2000)}]\n".encode(), option=RT
    )
    before = doc.to_yaml()
    with pytest.raises(ValueError, match="too many nodes"):
        doc["copy"] = doc["refs"]
    assert doc.to_yaml() == before


def test_several_views_in_one_value_share_one_budget():
    """Views that each fit the budget raise together when the value as a whole does not."""
    base = "[" + ", ".join(str(item) for item in range(512)) + "]"
    doc = yamlrocks.loads(
        f"base: &b {base}\nrefs: [{', '.join(['*b'] * 700)}]\n".encode(), option=RT
    )
    doc["one"] = doc["refs"]
    with pytest.raises(ValueError, match="too many nodes"):
        doc["three"] = [doc["refs"], doc["refs"], doc["refs"]]
    assert "three" not in doc.to_dict()
    assert len(doc.to_dict()["one"]) == 700


def test_the_documents_of_a_copied_stream_share_one_budget():
    """A stream whose documents each fit the budget raises when copied whole."""
    base = "[" + ", ".join(str(item) for item in range(512)) + "]"
    one = f"base: &b {base}\nrefs: [{', '.join(['*b'] * 700)}]\n"
    stream = yamlrocks.loads("---\n".join([one] * 3).encode(), option=RT)
    target = yamlrocks.loads(b"dst: 0\n", option=RT)
    with pytest.raises(ValueError, match="too many nodes"):
        target["dst"] = stream
    assert target.to_yaml() == b"dst: 0\n"


def test_a_document_with_many_aliases_still_copies():
    """A legitimate document with a few thousand aliased nodes copies in full."""
    base = "{" + ", ".join(f"k{item}: {item}" for item in range(10)) + "}"
    refs = "\n".join(f"  r{item}: *b" for item in range(300))
    doc = yamlrocks.loads(f"base: &b {base}\nrefs:\n{refs}\n".encode(), option=RT)
    doc["copy"] = doc["refs"]
    copied = yamlrocks.loads(doc.to_yaml())["copy"]
    assert copied == {
        f"r{item}": {f"k{key}": key for key in range(10)} for item in range(300)
    }


def test_a_copy_does_not_read_targets_the_anchor_map_truncated():
    """A small alias copies in full next to anchors that exhaust the anchor map's own budget."""
    for _ in range(3):
        doc = yamlrocks.loads(
            alias_bomb(8) + b"small: &s [1, 2, 3]\nholder: {r: *s}\n", option=RT
        )
        doc["copy"] = doc["holder"]
        assert doc.to_dict()["copy"] == {"r": [1, 2, 3]}


def test_a_long_alias_chain_copies():
    """A chain of 150 aliases, each nested in the next anchor, copies in full."""
    lines = ["a0: &a0 [x]"] + [
        f"a{level}: &a{level} [*a{level - 1}]" for level in range(1, 150)
    ]
    doc = yamlrocks.loads(("\n".join(lines) + "\n").encode(), option=RT)
    doc["copy"] = doc["a149"]
    assert yamlrocks.loads(doc.to_yaml())["copy"] == doc.to_dict()["a149"]


def test_a_deeply_nested_view_copies_or_raises_the_depth_error():
    """A view nested near the composer's limit copies; one deeper than the copy allows raises."""
    doc = yamlrocks.loads(b"r: " + b"[" * 900 + b"]" * 900 + b"\n", option=RT)
    doc["c"] = doc["r"]
    assert yamlrocks.loads(doc.to_yaml())["c"] == doc.to_dict()["r"]

    doc = yamlrocks.loads(b"r: " + b"[" * 990 + b"]" * 990 + b"\n", option=RT)
    wrapped = doc["r"]
    for _ in range(12):
        wrapped = [wrapped]
    with pytest.raises(ValueError, match="too deeply nested"):
        doc["c"] = wrapped


_SMALL_STACK_COPY = """
import threading, yamlrocks
RT = yamlrocks.OPT_ROUND_TRIP
results = []
def work():
    doc = yamlrocks.loads(b"r: " + b"[" * 900 + b"]" * 900 + b"\\n", option=RT)
    doc["c"] = doc["r"]
    doc.to_yaml()
    deep = yamlrocks.loads(b"r: " + b"[" * 995 + b"]" * 995 + b"\\n", option=RT)
    try:
        deep["c"] = [[[[[[deep["r"]]]]]]]
    except ValueError:
        results.append("depth error")
threading.stack_size(256 * 1024)
thread = threading.Thread(target=work)
thread.start()
thread.join()
print("OK", *results)
"""


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX thread stack sizing")
def test_a_deep_copy_survives_a_small_thread_stack():
    """Copying a deeply nested view on a 256 KB thread stack neither overflows nor aborts."""
    proc = subprocess.run(
        [sys.executable, "-c", _SMALL_STACK_COPY], capture_output=True, timeout=60
    )
    assert proc.returncode == 0, proc.stderr.decode()[:400]
    assert proc.stdout.strip() == b"OK depth error"


# -- Key comparisons and removal -------------------------------------------------


class FailingKey(str):
    """A `str` subclass whose comparison raises."""

    def __eq__(self, other):
        raise ValueError("comparison failed")

    __hash__ = str.__hash__


@pytest.mark.parametrize("operation", ["set", "get", "get with default", "delete"])
def test_a_key_comparison_that_raises_is_raised_and_changes_nothing(operation):
    """A key whose `__eq__` raises propagates the error from every operation, with no edit made."""
    doc = yamlrocks.loads(b"x: 1\ny: 2\n", option=RT)
    before = doc.to_yaml()
    key = FailingKey("x")
    with pytest.raises(ValueError, match="comparison failed"):
        if operation == "set":
            doc[key] = 5
        elif operation == "get":
            doc[key]
        elif operation == "get with default":
            doc.get(key, "default")
        else:
            del doc[key]
    assert doc.to_yaml() == before


def test_get_still_returns_the_default_for_a_missing_key():
    """`get()` returns its default when the key is simply not there."""
    doc = yamlrocks.loads(b"x: 1\n", option=RT)
    assert doc.get("missing", "default") == "default"
    assert doc.get(7, "default") == "default"
    assert doc.get(["a", "list"], "default") == "default"


def test_a_null_key_is_replaced_and_removed_rather_than_duplicated():
    """Assigning `doc[None]` twice keeps one null key, and `del doc[None]` removes it."""
    doc = yamlrocks.loads(b"a: 1\n", option=RT)
    doc[None] = 1
    doc[None] = 2
    assert doc.to_dict() == {"a": 1, None: 2}
    del doc[None]
    assert doc.to_dict() == {"a": 1}


def test_deleting_a_key_removes_every_duplicate_and_nothing_else():
    """`del doc[1]` removes all interleaved duplicates of the key in one pass."""
    doc = yamlrocks.loads(
        "".join(f"1: a{item}\nk{item}: b{item}\n" for item in range(2000)).encode(),
        option=RT,
    )
    del doc[1]
    assert doc.to_dict() == {f"k{item}": f"b{item}" for item in range(2000)}
