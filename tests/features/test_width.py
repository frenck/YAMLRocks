"""Best-effort line wrapping via the ``width`` argument to ``dumps``/``dump``.

The non-negotiable invariant is data fidelity: a fold may never change the
decoded value, so ``loads(dumps(x, width=w)) == x`` for every case here. Folding
is a soft limit, so a run of spaces or a break-free span may still exceed the
width.
"""

from __future__ import annotations

import pytest

import yamlrocks


def _max_line(data: bytes) -> int:
    return max((len(line) for line in data.decode().rstrip().split("\n")), default=0)


def test_default_does_not_wrap():
    """Without width, long lines are emitted verbatim (today's behavior)."""
    obj = {"msg": "one two three four five six seven eight nine ten eleven twelve"}
    assert _max_line(yamlrocks.dumps(obj)) > 60


def test_plain_scalar_wraps_and_round_trips():
    """A long plain scalar folds at spaces and reloads unchanged."""
    obj = {"msg": "one two three four five six seven eight nine ten eleven twelve"}
    out = yamlrocks.dumps(obj, width=30)
    assert yamlrocks.loads(out) == obj
    assert _max_line(out) <= 30


def test_quoted_scalar_wraps_and_round_trips():
    """A value that must be quoted folds inside the quotes and reloads."""
    obj = {"v": 'a "quoted" value that is plenty long enough to wrap a few times over'}
    out = yamlrocks.dumps(obj, width=30)
    assert yamlrocks.loads(out) == obj
    assert b'"' in out  # it really is quoted
    assert _max_line(out) <= 30


def test_flow_collection_wraps_and_round_trips():
    """Flow collections break after commas (whitespace there is insignificant)."""
    obj = {"nums": list(range(1, 40))}
    out = yamlrocks.dumps(obj, width=30, option=yamlrocks.OPT_FLOW_STYLE)
    assert yamlrocks.loads(out) == obj
    assert b"\n" in out  # it actually wrapped


def test_runs_of_spaces_are_never_split():
    """A fold never lands inside a run of spaces, which would drop one."""
    obj = {"x": "alpha   beta gamma   delta epsilon zeta eta theta iota kappa lambda"}
    out = yamlrocks.dumps(obj, width=20)
    # The triple-space runs survive intact under a 1.2 read.
    assert yamlrocks.loads(out) == obj


def test_unbreakable_token_exceeds_width():
    """A long no-space token cannot fold, so its line exceeds the width."""
    url = "https://example.com/a/very/long/path/that/has/no/spaces/to/break/at"
    obj = {"u": url}
    out = yamlrocks.dumps(obj, width=30)
    assert yamlrocks.loads(out) == obj
    assert _max_line(out) > 30  # soft limit: it could not be wrapped


def test_nested_continuation_indent_is_deeper():
    """Folded continuation lines indent under their key, not at the root."""
    obj = {"a": {"b": "deeply nested text that is long enough to wrap a couple times"}}
    out = yamlrocks.dumps(obj, width=30)
    assert yamlrocks.loads(out) == obj


@pytest.mark.parametrize("width", [10, 20, 40, 80])
@pytest.mark.parametrize(
    "obj",
    [
        {"description": "the quick brown fox jumps over the lazy dog " * 4},
        {"items": ["a longish phrase here"] * 8},
        {"mix": {"k": "v " * 30, "list": list(range(50))}},
        {"keyword_like": "true false yes no null on off " * 5},
        {"nums": list(range(100))},
    ],
)
def test_fidelity_matrix(obj, width):
    """Across widths and shapes, the decoded value is always preserved."""
    for option in (0, yamlrocks.OPT_FLOW_STYLE):
        out = yamlrocks.dumps(obj, width=width, option=option)
        assert yamlrocks.loads(out) == obj


def test_dump_forwards_width(tmp_path):
    """dump() forwards width to the file/stream path."""
    path = tmp_path / "out.yaml"
    obj = {"msg": "one two three four five six seven eight nine ten eleven twelve"}
    yamlrocks.dump(obj, str(path), width=30)
    written = path.read_bytes()
    assert yamlrocks.loads(written) == obj
    assert _max_line(written) <= 30


def test_compat_safe_dump_honors_explicit_width():
    """The PyYAML shim forwards an explicit width (its default stays unwrapped)."""
    import yamlrocks.compat as compat

    obj = {"msg": "one two three four five six seven eight nine ten eleven twelve"}
    wrapped = compat.safe_dump(obj, width=30)
    assert _max_line(wrapped.encode()) <= 30
    # Default leaves it on one line.
    assert _max_line(compat.safe_dump(obj).encode()) > 60


def test_round_trip_mode_ignores_width():
    """Round-trip preserves layout byte-for-byte; width does not apply there."""
    src = b"msg: one two three four five six seven eight nine ten eleven twelve\n"
    doc = yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP)
    assert doc.to_yaml() == src


def test_wrapped_plain_output_is_yamllint_line_length_clean():
    """A width-80 dump of wrappable text passes yamllint's line-length rule."""
    config = pytest.importorskip("yamllint.config")
    linter = pytest.importorskip("yamllint.linter")

    obj = {
        "description": "the quick brown fox jumps over the lazy dog " * 6,
        "summary": "another reasonably long sentence with plenty of spaces to fold at",
    }
    text = yamlrocks.dumps(obj, width=80).decode()
    cfg = config.YamlLintConfig("extends: default\nrules:\n  document-start: disable\n")
    problems = [
        f"{p.line}:{p.column} {p.rule}: {p.desc}"
        for p in linter.run(text, cfg)
        if p.level == "error"
    ]
    assert problems == [], problems
