"""Gate for the yamlrocks-vs-PyYAML differential oracle.

The heavy lifting (loading both, triaging the intentional 1.1-vs-1.2 gaps) lives
in :mod:`pyyaml_differential`, which doubles as a CLI for extended,
mutation-based runs. This file keeps two fast, deterministic checks in CI:

* on a curated set of schema-neutral documents the two libraries must agree
  exactly, so a regression that changes a common decode result fails here; and
* the oracle itself stays healthy over the YAML test suite corpus (it runs, and
  matches dominate), so the tool cannot silently rot.

The full corpus/generative run is a triage tool, not a unit gate: PyYAML
implements YAML 1.1 and is simply wrong on some 1.2 inputs (see the module
docstring), so a value mismatch there is triaged by hand, not asserted.
"""

from __future__ import annotations

import pytest
import pyyaml_differential as diff

yaml = pytest.importorskip("yaml")


# Documents whose decoded value is identical under YAML 1.1 and 1.2: no booleans
# from the 1.1-only set, no timestamps, no tags. yamlrocks and PyYAML must agree.
_AGREE = [
    b"key: value\n",
    b"a: 1\nb: 2\nc: 3\n",
    b"nested:\n  inner:\n    leaf: 42\n",
    b"list:\n  - 1\n  - 2\n  - 3\n",
    b"mixed:\n  - name: alice\n    age: 30\n  - name: bob\n    age: 25\n",
    b"flow_map: {a: 1, b: 2}\n",
    b"flow_list: [1, 2, 3]\n",
    b"scalars:\n  i: 42\n  f: 3.14\n  s: hello\n  t: true\n  e: false\n  n: null\n",
    b"quoted:\n  single: 'value'\n  double: \"value\"\n",
    b"empty_map: {}\nempty_list: []\n",
    b"block: |\n  line one\n  line two\n",
    b"folded: >\n  one two\n  three\n",
    b"deep:\n  - {a: [1, 2], b: {c: 3}}\n  - [x, y, z]\n",
]


@pytest.mark.parametrize("doc", _AGREE)
def test_agrees_with_pyyaml_on_schema_neutral_docs(doc):
    """On documents where 1.1 and 1.2 resolve identically, the two libraries
    decode to the same value."""
    assert diff.compare(doc).outcome == diff.Outcome.MATCH


def test_triage_absorbs_known_intentional_gaps():
    """The triage table treats the documented 1.2-core differences (timestamps,
    NaN) as matches, not mismatches, so they never masquerade as bugs."""
    for doc in (b"when: 2001-12-15T02:59:43Z\n", b"on: 2002-12-14\n", b"x: .nan\n"):
        assert diff.compare(doc).outcome == diff.Outcome.MATCH


def test_oracle_stays_healthy_over_the_test_suite():
    """The oracle runs over the whole YAML test suite corpus without error, and
    genuine matches dominate. This guards the tool itself, not yamlrocks: the
    remaining value mismatches are PyYAML's 1.1 divergences, triaged by hand."""
    inputs = diff.yaml_test_suite_inputs()
    if not inputs:
        pytest.skip("YAML test suite submodule not checked out")
    counts: dict[str, int] = {}
    for raw in inputs:
        outcome = diff.compare(raw).outcome
        counts[outcome] = counts.get(outcome, 0) + 1
    assert counts.get(diff.Outcome.MATCH, 0) > counts.get(
        diff.Outcome.VALUE_MISMATCH, 0
    )


def test_generator_is_reproducible_and_runnable():
    """The mutation generator is seeded (reproducible) and every input it emits
    flows through the oracle without raising."""
    batch = diff.generate(200, seed=1234)
    assert batch == diff.generate(200, seed=1234)
    for raw in batch:
        diff.compare(raw)  # must not raise, whatever the verdict
