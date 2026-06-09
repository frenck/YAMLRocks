"""yamllint compliance: assert ``dumps`` output passes the popular YAML linter.

These tests lock in the guarantee that yamlrocks emits *yamllint-clean* YAML by
default. They run [yamllint](https://yamllint.readthedocs.io/) over the bytes
``dumps`` produces for a spread of structures and option combinations, so any
future emitter change that breaks style compliance fails here.

Two of yamllint's default rules reflect deliberate yamlrocks choices rather than
emitter bugs, so they are configured rather than chased:

* ``document-start`` (require ``---``) is opt-in via ``OPT_EXPLICIT_START``; a
  dedicated test below proves that flag satisfies the rule.
* ``line-length`` (wrap at 80) is disabled: yamlrocks does not wrap long scalars
  (predictable, diff- and grep-friendly output).

yamllint's default ``indent-sequences: true`` matches the yamlrocks default, so
default output passes as-is; the ``OPT_INDENTLESS_SEQUENCES`` style is checked
against the matching ``indent-sequences: false`` configuration.

The suite skips cleanly when yamllint is not installed.
"""

from __future__ import annotations

import pytest

import yamlrocks

yamllint_config = pytest.importorskip("yamllint.config")
yamllint_linter = pytest.importorskip("yamllint.linter")


def _lint(content: bytes, extra_rules: str = "") -> list[str]:
    """Run yamllint over ``content`` and return a list of problem descriptions.

    The two rules that reflect deliberate yamlrocks choices are disabled:
    ``line-length`` (we do not wrap) and ``document-start`` (the ``---`` marker is
    opt-in via ``OPT_EXPLICIT_START``). ``extra_rules`` appends further overrides.
    """
    rules = "  line-length: disable\n  document-start: disable\n" + extra_rules
    config = yamllint_config.YamlLintConfig(f"extends: default\nrules:\n{rules}")
    text = content.decode("utf-8")
    return [
        f"{p.line}:{p.column} {p.level} {p.desc} ({p.rule})"
        for p in yamllint_linter.run(text, config)
    ]


# Representative structures covering scalars, nesting, sequences, sequences of
# mappings, ambiguous strings (which get quoted), unicode, and an empty value.
SAMPLES = [
    {"name": "app", "version": "1.0", "enabled": True, "count": 3, "note": None},
    {"server": {"host": "localhost", "ports": [80, 443], "tags": ["web", "prod"]}},
    {"items": [{"a": 1, "b": "yes"}, {"a": 2, "b": "no"}]},
    {"matrix": [[1, 2], [3, 4]], "flat": ["x", "y", "z"]},
    {"plain": "hello world", "ambiguous": "true", "version": "1.2.3", "café": "☕"},
    [{"step": "build"}, {"step": "test"}, {"step": "deploy"}],
    {"empty_map": {}, "empty_list": [], "blank": None},
]

# Option combinations whose output stays clean under yamllint's *default* rules
# (these do not change sequence indentation, which yamllint's default requires).
DEFAULT_CLEAN_OPTIONS = [
    0,
    yamlrocks.OPT_SORT_KEYS,
    yamlrocks.OPT_SINGLE_QUOTES,
    yamlrocks.OPT_NULL_AS_KEYWORD,
    yamlrocks.OPT_EXPLICIT_START | yamlrocks.OPT_EXPLICIT_END,
]


@pytest.mark.parametrize("data", SAMPLES)
@pytest.mark.parametrize("option", DEFAULT_CLEAN_OPTIONS)
def test_dumps_is_yamllint_clean(data, option):
    """Default-style ``dumps`` output passes yamllint's default rules."""
    out = yamlrocks.dumps(data, option=option)
    problems = _lint(out)
    assert not problems, (
        f"yamllint flagged dumps output:\n{out.decode()}\n" + "\n".join(problems)
    )


def test_dumps_literal_block_is_yamllint_clean():
    """Multi-line strings (literal block scalars by default) are yamllint-clean."""
    out = yamlrocks.dumps({"script": "line one\nline two\nline three\n"})
    assert b"|" in out  # emitted as a literal block by default
    assert not _lint(out)


def test_explicit_start_satisfies_document_start():
    """``OPT_EXPLICIT_START`` makes output pass yamllint's ``document-start`` rule
    (which warns on a missing ``---``)."""
    out = yamlrocks.dumps({"a": 1, "b": [1, 2]}, option=yamlrocks.OPT_EXPLICIT_START)
    # Only line-length is disabled here; document-start stays enabled so the `---`
    # the flag emits is actually exercised.
    config = yamllint_config.YamlLintConfig(
        "extends: default\nrules:\n  line-length: disable\n"
    )
    problems = list(yamllint_linter.run(out.decode("utf-8"), config))
    assert not problems, "\n".join(str(p) for p in problems)


def test_indentless_sequences_pass_with_matching_config():
    """``OPT_INDENTLESS_SEQUENCES`` output is clean under yamllint configured for
    that style (``indent-sequences: false``)."""
    out = yamlrocks.dumps(
        {"ports": [80, 443], "tags": ["a", "b"]},
        option=yamlrocks.OPT_INDENTLESS_SEQUENCES,
    )
    assert not _lint(out, extra_rules="  indentation: {indent-sequences: false}\n")
