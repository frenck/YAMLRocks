"""Implicit timestamp resolution (``OPT_TIMESTAMPS`` and ``OPT_PYYAML_COMPAT``).

YAML 1.2's core schema has no timestamp type, so a date/datetime scalar is a
plain string by default. ``OPT_TIMESTAMPS`` opts into PyYAML-style resolution
under any schema; ``OPT_PYYAML_COMPAT`` implies it. Only plain scalars resolve
(a quoted one stays a string), and an out-of-range value falls back to a string
rather than raising.
"""

from __future__ import annotations

import datetime as dt

import pytest

import yamlrocks

TS = yamlrocks.OPT_TIMESTAMPS
CO = yamlrocks.OPT_PYYAML_COMPAT
V11 = yamlrocks.OPT_YAML_1_1
RT = yamlrocks.OPT_ROUND_TRIP
AN = yamlrocks.OPT_ANNOTATED


def _v(src: bytes, option: int | None = None):
    return (
        yamlrocks.loads(src, option=option)["v"]
        if option
        else yamlrocks.loads(src)["v"]
    )


def test_default_leaves_timestamps_as_strings():
    """Without the flag, a date/datetime scalar is a plain string (YAML 1.2)."""
    assert _v(b"v: 2024-01-15") == "2024-01-15"
    assert _v(b"v: 2024-01-15T13:30:45") == "2024-01-15T13:30:45"


def test_opt_timestamps_resolves_date_and_datetime():
    """OPT_TIMESTAMPS resolves a plain date to date and datetime to datetime."""
    assert _v(b"v: 2024-01-15", TS) == dt.date(2024, 1, 15)
    assert _v(b"v: 2024-01-15T13:30:45", TS) == dt.datetime(2024, 1, 15, 13, 30, 45)


def test_pyyaml_compat_implies_timestamps():
    """OPT_PYYAML_COMPAT resolves timestamps without needing OPT_TIMESTAMPS."""
    assert _v(b"v: 2024-01-15", CO) == dt.date(2024, 1, 15)


def test_timezones_are_aware():
    """`Z` and an explicit offset produce timezone-aware datetimes."""
    assert _v(b"v: 2024-01-15T13:30:45Z", TS) == dt.datetime(
        2024, 1, 15, 13, 30, 45, tzinfo=dt.UTC
    )
    assert _v(b"v: 2024-01-15T13:30:45+05:30", TS) == dt.datetime(
        2024, 1, 15, 13, 30, 45, tzinfo=dt.timezone(dt.timedelta(hours=5, minutes=30))
    )


def test_fractional_seconds_truncate_to_microseconds():
    """Fractional seconds beyond microsecond precision are truncated, like PyYAML."""
    assert _v(b"v: 2024-01-15T13:30:45.123456789", TS) == dt.datetime(
        2024, 1, 15, 13, 30, 45, 123456
    )


def test_naive_datetime_when_no_zone():
    """A datetime with no zone is naive (tzinfo is None)."""
    assert _v(b"v: 2024-01-15T13:30:45", TS).tzinfo is None


def test_quoted_timestamp_stays_a_string():
    """A quoted scalar is explicitly a string, so it never resolves (unlike yaml-rs)."""
    assert _v(b'v: "2024-01-15"', TS) == "2024-01-15"
    assert _v(b"v: '2024-01-15T13:30:45'", CO) == "2024-01-15T13:30:45"


def test_tagged_timestamp_stays_a_string():
    """An explicit tag suppresses implicit resolution."""
    assert _v(b"v: !!str 2024-01-15", TS) == "2024-01-15"


@pytest.mark.parametrize(
    "text",
    [
        b"v: 2024-1-5",  # date-only needs two-digit month/day
        b"v: 2024-13-01",  # month out of range (PyYAML would crash; we keep a string)
        b"v: 2024-02-30",  # not a real calendar date
        b"v: 2024-01-15T25:00:00",  # hour out of range
        b"v: not-a-date",
    ],
)
def test_non_timestamps_and_invalid_fall_back_to_strings(text):
    """A non-timestamp, or a timestamp-shaped but out-of-range value, stays a string."""
    assert isinstance(_v(text, TS), str)


def test_strict_yaml_1_1_does_not_resolve_timestamps():
    """Strict YAML 1.1 keeps its own semantics: dates stay strings, `13:30:45` is
    a sexagesimal int; only compat or the explicit flag resolve timestamps."""
    assert _v(b"v: 2024-01-15", V11) == "2024-01-15"
    assert _v(b"v: 13:30:45", V11) == 48645


def test_round_trip_under_compat_resolves_and_stays_byte_identical():
    """In round-trip mode under compat, to_dict resolves dates while the source
    still re-emits byte-for-byte."""
    src = b"d: 2024-01-15\nt: 2024-01-15T13:30:45Z\n"
    doc = yamlrocks.loads(src, option=RT | CO)
    assert doc.to_dict() == {
        "d": dt.date(2024, 1, 15),
        "t": dt.datetime(2024, 1, 15, 13, 30, 45, tzinfo=dt.UTC),
    }
    assert doc["d"] == dt.date(2024, 1, 15)
    assert doc.to_yaml() == src


def test_round_trip_with_standalone_flag_keeps_strings():
    """The standalone OPT_TIMESTAMPS flag is a fast-path feature; in round-trip
    mode typed timestamps follow the schema, so without compat they stay strings."""
    doc = yamlrocks.loads(b"d: 2024-01-15\n", option=RT | TS)
    assert doc.to_dict()["d"] == "2024-01-15"


def test_annotated_compat_gives_plain_date_without_line():
    """Annotated mode under compat resolves to a plain date/datetime, which carries
    no source location (documented)."""
    data = yamlrocks.loads(b"d: 2024-01-15\n", option=AN | CO)
    assert data["d"] == dt.date(2024, 1, 15)
    assert not hasattr(data["d"], "__line__")


def test_dumped_date_round_trips_under_compat():
    """A Python date/datetime dumps to a timestamp scalar that reloads to the same
    value under compat."""
    for obj in [
        dt.date(2024, 1, 15),
        dt.datetime(2024, 1, 15, 13, 30, 45),
        dt.datetime(2024, 1, 15, 13, 30, 45, tzinfo=dt.UTC),
    ]:
        out = yamlrocks.dumps({"v": obj})
        assert yamlrocks.loads(out, option=CO)["v"] == obj
