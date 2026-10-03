"""Width-aware representation uses the shared native scalar folding logic."""

import pytest

import yamlrocks


@pytest.mark.parametrize("style", ["plain", "single", "double"])
@pytest.mark.parametrize(
    "value",
    [
        "alpha beta gamma delta",
        "a  b c d",
        "a ! b c ? d",
        "café naïve voilà",
        "x \\ y z",
    ],
)
def test_scalar_folding(style, value):
    """Folding preserves scalar content for each inline representation style."""

    def represent(obj):
        if obj == value:
            return yamlrocks.YAMLRocksScalar(value, style=style)
        return None

    output = yamlrocks.dumps({"k": value}, represent=represent, width=12)
    assert yamlrocks.loads(output) == {"k": value}


def test_keys_remain_on_one_line():
    """A small width must not break implicit mapping-key syntax."""
    value = {"a very long key with spaces": "a long value with spaces"}
    output = yamlrocks.dumps(value, width=8, represent=lambda _: None)
    assert output.startswith(b"a very long key with spaces:")
    assert yamlrocks.loads(output) == value


def test_flow_sequence_wraps_between_entries():
    """Flow separators offer line breaks even for values without spaces."""
    data = ["alpha", "beta", "gamma", "delta"]
    output = yamlrocks.dumps(
        data,
        represent=lambda obj: (
            yamlrocks.YAMLRocksSequence(obj, flow=True)
            if isinstance(obj, list)
            else None
        ),
        width=12,
    )
    assert len(output.splitlines()) > 1
    assert yamlrocks.loads(output) == data


def test_zero_width_is_unlimited():
    """Width zero preserves the previous unwrapped emission behavior."""
    data = {"k": "alpha beta gamma delta"}
    assert yamlrocks.dumps(data, width=0, represent=lambda _: None).count(b"\n") == 1
