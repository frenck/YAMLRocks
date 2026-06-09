"""Generative robustness ("fuzz") tests.

Two complementary checks, both bounded by the suite-wide memory cap:

* random *valid* Python structures must survive a dump → load round-trip and a
  round-trip-mode emit → load cycle;
* random *arbitrary* byte and text inputs must never crash or hang the parser -
  ``loads`` either returns a value or raises a catchable error.

The generators are seeded for reproducibility.
"""

from __future__ import annotations

import random
import string

import pytest

import yamlrocks

ATOMS = [
    None,
    True,
    False,
    0,
    1,
    -1,
    42,
    255,
    -9999,
    3.14,
    -0.5,
    1e10,
    "",
    "hello",
    "with spaces",
    "café",
    "1.2.3",
    "true",
    "null",
    "123",
    "a: b",
    "multi\nline",
]


def random_structure(rng: random.Random, depth: int = 0):
    if depth >= 4 or rng.random() < 0.4:
        return rng.choice(ATOMS)
    if rng.random() < 0.5:
        return [random_structure(rng, depth + 1) for _ in range(rng.randint(0, 4))]
    return {
        f"key{i}": random_structure(rng, depth + 1) for i in range(rng.randint(0, 4))
    }


def normalize(value):
    """NaN-safe deep comparison key (floats compare by repr)."""
    if isinstance(value, float):
        return ("f", repr(value))
    if isinstance(value, bool):
        return ("b", value)
    if isinstance(value, dict):
        return ("d", tuple((k, normalize(v)) for k, v in value.items()))
    if isinstance(value, list):
        return ("l", tuple(normalize(v) for v in value))
    return ("a", type(value).__name__, value)


@pytest.mark.parametrize("seed", range(60))
def test_random_structures_roundtrip(seed):
    """Random valid structures survive fast-path and round-trip-mode cycles."""
    rng = random.Random(seed)
    obj = {"root": random_structure(rng)}
    # Fast-path dump → load.
    assert normalize(yamlrocks.loads(yamlrocks.dumps(obj))) == normalize(obj)
    # Round-trip-mode emit → load.
    doc = yamlrocks.loads(yamlrocks.dumps(obj), option=yamlrocks.OPT_ROUND_TRIP)
    assert normalize(yamlrocks.loads(doc.to_yaml())) == normalize(obj)


@pytest.mark.parametrize("seed", range(200))
def test_random_bytes_do_not_crash(seed):
    """Random byte inputs either load or raise a catchable error without crashing."""
    rng = random.Random(seed + 1000)
    n = rng.randint(0, 60)
    # Bias toward YAML-significant characters to exercise the scanner.
    alphabet = string.ascii_letters + string.digits + " \n\t:-#{}[],&*!|>'\"%@`?~."
    data = "".join(rng.choice(alphabet) for _ in range(n)).encode()
    try:
        yamlrocks.loads(data)
    except (yamlrocks.YAMLRocksDecodeError, ValueError, TypeError):
        pass


@pytest.mark.parametrize("seed", range(100))
def test_random_unicode_text_does_not_crash(seed):
    """Random unicode text inputs either load or raise a catchable error without crashing."""
    rng = random.Random(seed + 5000)
    n = rng.randint(0, 40)
    text = "".join(chr(rng.randint(1, 0x2FFF)) for _ in range(n))
    try:
        yamlrocks.loads(text)
    except (yamlrocks.YAMLRocksDecodeError, ValueError, TypeError):
        pass
