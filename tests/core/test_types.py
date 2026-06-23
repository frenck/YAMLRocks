"""Serialization of standard-library and numpy types."""

from __future__ import annotations

import dataclasses
import datetime
import decimal
import enum
import pathlib
import uuid

import pytest

import yamlrocks


def _dump_load(obj, *, option=None):
    return yamlrocks.loads(yamlrocks.dumps(obj, option=option))


def test_datetime():
    """datetime serializes to an ISO 8601 string."""
    out = yamlrocks.dumps({"t": datetime.datetime(2026, 1, 2, 15, 30, 45)})
    assert out == b"t: 2026-01-02T15:30:45\n"


def test_date():
    """date serializes to an ISO 8601 date string."""
    assert _dump_load({"d": datetime.date(2026, 1, 2)}) == {"d": "2026-01-02"}


def test_time():
    """time serializes to an ISO 8601 time string."""
    assert _dump_load({"t": datetime.time(15, 30)}) == {"t": "15:30:00"}


def test_datetime_microseconds_included_by_default():
    """The microsecond field is serialized by default."""
    dt = datetime.datetime(2026, 1, 2, 15, 30, 45, 123456)
    assert yamlrocks.dumps({"t": dt}) == b"t: 2026-01-02T15:30:45.123456\n"


def test_opt_omit_microseconds():
    """OPT_OMIT_MICROSECONDS drops the microsecond field."""
    dt = datetime.datetime(2026, 1, 2, 15, 30, 45, 123456)
    out = yamlrocks.dumps({"t": dt}, option=yamlrocks.OPT_OMIT_MICROSECONDS)
    assert out == b"t: 2026-01-02T15:30:45\n"


def test_opt_omit_microseconds_on_time():
    """OPT_OMIT_MICROSECONDS also applies to a bare time."""
    t = datetime.time(15, 30, 45, 500000)
    out = yamlrocks.dumps({"t": t}, option=yamlrocks.OPT_OMIT_MICROSECONDS)
    assert out == b"t: 15:30:45\n"


def test_opt_naive_utc():
    """OPT_NAIVE_UTC tags a naive datetime with a +00:00 offset."""
    dt = datetime.datetime(2026, 1, 2, 15, 30, 45)
    out = yamlrocks.dumps({"t": dt}, option=yamlrocks.OPT_NAIVE_UTC)
    assert out == b"t: 2026-01-02T15:30:45+00:00\n"


def test_opt_naive_utc_leaves_aware_datetime_untouched():
    """OPT_NAIVE_UTC does not alter an already timezone-aware datetime."""
    dt = datetime.datetime(
        2026, 1, 2, 15, 30, 45, tzinfo=datetime.timezone(datetime.timedelta(hours=2))
    )
    out = yamlrocks.dumps({"t": dt}, option=yamlrocks.OPT_NAIVE_UTC)
    assert out == b"t: 2026-01-02T15:30:45+02:00\n"


def test_opt_naive_utc_skips_plain_date():
    """OPT_NAIVE_UTC only applies to datetimes, not to a plain date."""
    out = yamlrocks.dumps(
        {"d": datetime.date(2026, 1, 2)}, option=yamlrocks.OPT_NAIVE_UTC
    )
    assert yamlrocks.loads(out) == {"d": "2026-01-02"}


def test_opt_utc_z():
    """OPT_UTC_Z renders a UTC offset as Z."""
    dt = datetime.datetime(2026, 1, 2, 15, 30, 45, tzinfo=datetime.UTC)
    out = yamlrocks.dumps({"t": dt}, option=yamlrocks.OPT_UTC_Z)
    assert out == b"t: 2026-01-02T15:30:45Z\n"


def test_opt_utc_z_with_naive_utc():
    """OPT_UTC_Z composes with OPT_NAIVE_UTC to emit Z for a naive datetime."""
    dt = datetime.datetime(2026, 1, 2, 15, 30, 45)
    out = yamlrocks.dumps(
        {"t": dt}, option=yamlrocks.OPT_UTC_Z | yamlrocks.OPT_NAIVE_UTC
    )
    assert out == b"t: 2026-01-02T15:30:45Z\n"


def test_opt_utc_z_leaves_non_utc_offset():
    """OPT_UTC_Z only rewrites +00:00; other offsets are untouched."""
    dt = datetime.datetime(
        2026, 1, 2, 15, 30, 45, tzinfo=datetime.timezone(datetime.timedelta(hours=5))
    )
    out = yamlrocks.dumps({"t": dt}, option=yamlrocks.OPT_UTC_Z)
    assert out == b"t: 2026-01-02T15:30:45+05:00\n"


def test_opt_passthrough_datetime():
    """OPT_PASSTHROUGH_DATETIME routes datetimes to the default callback."""
    dt = datetime.datetime(2026, 1, 2, 15, 30, 45)

    def default(obj):
        assert isinstance(obj, datetime.datetime)
        return "handled"

    out = yamlrocks.dumps(
        {"t": dt}, default=default, option=yamlrocks.OPT_PASSTHROUGH_DATETIME
    )
    assert out == b"t: handled\n"


def test_opt_passthrough_datetime_covers_date_and_time():
    """OPT_PASSTHROUGH_DATETIME also passes date and time to default."""
    seen = []

    def default(obj):
        seen.append(type(obj).__name__)
        return "x"

    yamlrocks.dumps(
        {"d": datetime.date(2026, 1, 2), "t": datetime.time(1, 2)},
        default=default,
        option=yamlrocks.OPT_PASSTHROUGH_DATETIME,
    )
    assert set(seen) == {"date", "time"}


def test_opt_passthrough_dataclass():
    """OPT_PASSTHROUGH_DATACLASS routes dataclass instances to default."""

    @dataclasses.dataclass
    class Point:
        x: int
        y: int

    def default(obj):
        assert isinstance(obj, Point)
        return {"point": [obj.x, obj.y]}

    raw = yamlrocks.dumps(
        Point(1, 2), default=default, option=yamlrocks.OPT_PASSTHROUGH_DATACLASS
    )
    assert yamlrocks.loads(raw) == {"point": [1, 2]}


def test_dataclass_serialized_without_passthrough():
    """Without the flag a dataclass still serializes to a mapping."""

    @dataclasses.dataclass
    class Point:
        x: int
        y: int

    assert _dump_load(Point(1, 2)) == {"x": 1, "y": 2}


def test_uuid():
    """UUID serializes to its string form."""
    u = uuid.UUID("12345678-1234-5678-1234-567812345678")
    assert _dump_load({"id": u}) == {"id": str(u)}


def test_decimal_is_number():
    """Decimal serializes as a numeric value."""
    assert _dump_load({"v": decimal.Decimal("3.14")}) == {"v": 3.14}


def test_pathlib_path():
    """pathlib.Path serializes to its string representation."""
    # str(Path) is OS-native (forward slashes on POSIX, backslashes on Windows),
    # so compare against the platform's own rendering rather than a literal.
    path = pathlib.Path("/etc/config")
    assert _dump_load({"p": path}) == {"p": str(path)}


def test_plain_enum_uses_value():
    """A plain Enum serializes using its member value."""

    class Color(enum.Enum):
        RED = "red"
        GREEN = "green"

    assert _dump_load({"c": Color.RED}) == {"c": "red"}


def test_int_enum_is_int():
    """An IntEnum serializes as its integer value."""

    class Level(enum.IntEnum):
        LOW = 1
        HIGH = 9

    assert _dump_load({"l": Level.HIGH}) == {"l": 9}


def test_dataclass_is_mapping():
    """A dataclass serializes as a mapping of its fields."""

    @dataclasses.dataclass
    class Point:
        x: int
        y: int

    assert _dump_load({"p": Point(1, 2)}) == {"p": {"x": 1, "y": 2}}


def test_tuple_is_sequence():
    """A tuple serializes as a sequence."""
    assert _dump_load({"t": (1, 2, 3)}) == {"t": [1, 2, 3]}


def test_set_is_sequence():
    """A frozenset serializes as a sequence."""
    assert _dump_load({"s": frozenset([1])}) == {"s": [1]}


def test_unsupported_type_still_errors():
    """An unsupported type raises YAMLRocksEncodeError."""
    with pytest.raises(yamlrocks.YAMLRocksEncodeError):
        yamlrocks.dumps({"x": object()})


def test_compact_block_mapping_as_explicit_key():
    """A compact block mapping used as an explicit key loads as a tuple of pairs.

    Spec example 8.19: ``? earth: blue`` is a block mapping ``{earth: blue}`` used
    as the key, with ``: moon: white`` a block mapping value. A mapping is not a
    hashable Python key, so it renders as a tuple of (key, value) pairs.
    """
    doc = yamlrocks.loads(b"- sun: yellow\n- ? earth: blue\n  : moon: white\n")
    assert doc == [
        {"sun": "yellow"},
        {(("earth", "blue"),): {"moon": "white"}},
    ]


def test_compact_block_sequence_as_explicit_key():
    """A compact block sequence used as an explicit key loads as a tuple."""
    doc = yamlrocks.loads(b"? - a\n  - b\n: value\n")
    assert doc == {("a", "b"): "value"}


def test_default_callback_still_works():
    """The default callback handles otherwise unsupported types."""
    out = yamlrocks.dumps({"x": object()}, default=lambda o: "fallback")
    assert yamlrocks.loads(out) == {"x": "fallback"}


def test_numpy_lookalike_module_is_not_serialized():
    """An object whose module merely starts with ``numpy`` (e.g. a
    ``numpycompat`` shim) is not mistaken for numpy and coerced via ``tolist``;
    detection uses a real type check, so it falls through to the encoder."""

    class FakeArray:
        __module__ = "numpycompat.fake"

        def tolist(self):  # would be called if the numpy path were taken
            return [1, 2, 3]

    with pytest.raises(yamlrocks.YAMLRocksEncodeError):
        yamlrocks.dumps({"x": FakeArray()}, option=yamlrocks.OPT_SERIALIZE_NUMPY)


# -- numpy (opt-in) ----------------------------------------------------------

np = pytest.importorskip("numpy")


def test_numpy_array():
    """A numpy array serializes to a list under OPT_SERIALIZE_NUMPY."""
    out = _dump_load({"a": np.array([1, 2, 3])}, option=yamlrocks.OPT_SERIALIZE_NUMPY)
    assert out == {"a": [1, 2, 3]}


def test_numpy_scalar():
    """A numpy scalar serializes to a plain int under OPT_SERIALIZE_NUMPY."""
    out = _dump_load({"x": np.int64(42)}, option=yamlrocks.OPT_SERIALIZE_NUMPY)
    assert out == {"x": 42}


def test_numpy_matrix():
    """A 2D numpy array serializes to nested lists under OPT_SERIALIZE_NUMPY."""
    out = _dump_load(
        {"m": np.array([[1, 2], [3, 4]])}, option=yamlrocks.OPT_SERIALIZE_NUMPY
    )
    assert out == {"m": [[1, 2], [3, 4]]}


def test_numpy_requires_flag():
    """Serializing numpy without the flag raises YAMLRocksEncodeError."""
    with pytest.raises(yamlrocks.YAMLRocksEncodeError):
        yamlrocks.dumps({"a": np.array([1])})
