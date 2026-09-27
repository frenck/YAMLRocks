"""Document-level presentation metadata."""

import pytest

import yamlrocks


@pytest.mark.parametrize("prefix", ["", "# header\n", "\ufeff", "%YAML 1.2\n"])
def test_explicit_start(prefix):
    """The parsed flag handles comments, BOMs, and YAML directives."""
    source = (prefix + "---\nkey: value\n").encode()
    doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)
    assert doc.explicit_start is True
    assert doc.to_yaml() == source
    doc["key"] = "changed"
    assert doc.explicit_start is True


def test_implicit_start():
    """An implicit document stays implicit, even if a scalar contains dashes."""
    doc = yamlrocks.loads(b"# header\nkey: '---'\n", option=yamlrocks.OPT_ROUND_TRIP)
    assert doc.explicit_start is False
    doc["key"] = "changed"
    assert doc.explicit_start is False


def test_empty_input_has_no_explicit_start():
    """A document wrapper with no root reports no start marker."""
    assert not yamlrocks.loads(b"", option=yamlrocks.OPT_ROUND_TRIP).explicit_start
