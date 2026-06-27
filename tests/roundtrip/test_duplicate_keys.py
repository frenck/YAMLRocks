"""Round-trip handling of duplicate mapping keys.

YAML allows the same key twice in a mapping; the decoder keeps the last value
(last-wins), exactly as ``loads`` and a Python ``dict`` would. The round-trip
cursor presents that same logical mapping: reads, ``locate``, item access,
``keys``, ``len``, set, and delete all act on the effective (last) entry, while
the source structure is still preserved for re-emission. A duplicate key is
unusual (and rejected outright under ``OPT_DUPLICATE_KEYS_ERROR``), but when one
slips through, the cursor must not disagree with ``loads``.
"""

from __future__ import annotations

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP
DUP = b"trigger: state\ntrigger: time\nname: app\n"


def test_loads_is_last_wins():
    """Baseline: the fast path keeps the last value of a duplicate key."""
    assert yamlrocks.loads(DUP) == {"trigger": "time", "name": "app"}


def test_read_resolves_to_the_last_occurrence():
    """Item access, the node cursor, and locate all read the effective value."""
    doc = yamlrocks.loads(DUP, option=RT)
    assert doc["trigger"] == "time"
    assert doc.node["trigger"].value == "time"
    assert doc.locate(["trigger"]).value == "time"


def test_locate_points_at_the_last_occurrence_position():
    """The located node's position is the effective (second) entry, not the first."""
    node = yamlrocks.loads(DUP, option=RT).locate(["trigger"])
    assert (node.line, node.column) == (2, 10)  # the value 'time' on line 2


def test_dict_keys_and_len_collapse_duplicates():
    """to_dict, keys(), and len() present one logical entry per key."""
    doc = yamlrocks.loads(DUP, option=RT)
    assert doc.to_dict() == {"trigger": "time", "name": "app"}
    assert doc.keys() == ["trigger", "name"]
    assert len(doc) == 2  # not 3: the duplicate 'trigger' counts once


def test_set_updates_the_effective_entry():
    """Assigning a duplicate key updates its last occurrence and re-reads back."""
    doc = yamlrocks.loads(DUP, option=RT)
    doc["trigger"] = "webhook"
    assert doc["trigger"] == "webhook"
    assert yamlrocks.loads(doc.to_yaml())["trigger"] == "webhook"


def test_delete_removes_every_occurrence():
    """Deleting a duplicate key removes it entirely (no shadowed entry resurfaces)."""
    doc = yamlrocks.loads(DUP, option=RT)
    del doc["trigger"]
    assert "trigger" not in doc
    assert doc.to_dict() == {"name": "app"}
    assert yamlrocks.loads(doc.to_yaml()) == {"name": "app"}


def test_keys_that_resolve_equal_collapse_like_to_dict():
    """Keys spelled differently but resolving equal (yes/true under 1.1) collapse."""
    y11 = RT | yamlrocks.OPT_YAML_1_1
    doc = yamlrocks.loads(b"yes: a\ntrue: b\n", option=y11)
    assert doc.to_dict() == {True: "b"}
    assert doc.keys() == [True]  # not [True, True]
    assert len(doc) == 1
    assert doc[True] == "b"


def test_delete_matches_by_resolved_key():
    """del uses the resolved key, like read/set: del doc[True] removes a yes: entry."""
    doc = yamlrocks.loads(b"yes: a\nname: x\n", option=RT | yamlrocks.OPT_YAML_1_1)
    del doc[True]
    assert doc.to_dict() == {"name": "x"}


def test_unique_keys_are_unaffected():
    """A normal mapping (no duplicates) behaves exactly as before."""
    doc = yamlrocks.loads(b"a: 1\nb: 2\n", option=RT)
    assert doc.keys() == ["a", "b"]
    assert len(doc) == 2
    assert doc.locate(["b"]).value == 2
