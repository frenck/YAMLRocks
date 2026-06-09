import json
import math
import pathlib

import yamlrocks


def jequal(a, b):
    if isinstance(a, float) and isinstance(b, float):
        return a == b or (math.isnan(a) and math.isnan(b))
    if isinstance(a, bool) or isinstance(b, bool):
        return a is b
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(jequal(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(
            jequal(x, y) for x, y in zip(a, b, strict=False)
        )
    return a == b


base = pathlib.Path("tests/data/yaml_test_suite/cases")
roundtrip_unstable, json_mismatch, rejected, error_accepted, parse_failures = (
    [],
    [],
    [],
    [],
    [],
)
for c in sorted(base.iterdir()):
    if not (c / "in.yaml").exists():
        # Skip non-case entries: the submodule's `.git` pointer and the
        # multi-document cases whose variants live in numbered subdirectories
        # (these have no top-level `in.yaml`), matching the test harness filter.
        continue
    inp = (c / "in.yaml").read_bytes()
    is_err = (c / "error").exists()
    has_json = (c / "in.json").exists()
    try:
        val = yamlrocks.loads(inp)
        parsed = True
    except Exception:
        parsed = False
    if is_err and not has_json and not parsed:
        # Correctly rejected an invalid document; nothing to round-trip.
        rejected.append(c.name)
        continue
    if is_err and parsed:
        # The suite marks this input invalid, but the pragmatic parser accepts
        # it. Track the laxness so it cannot grow silently and can only shrink.
        error_accepted.append(c.name)
    if not is_err and not parsed:
        # A valid document the pragmatic parser does not handle yet. Baselined
        # so the gap is visible and can only shrink as the parser improves.
        parse_failures.append(c.name)
    if parsed:
        # Round-trip must be byte-for-byte identical for an unmodified document.
        try:
            emitted = yamlrocks.loads(inp, option=yamlrocks.OPT_ROUND_TRIP).to_yaml()
            if emitted != inp:
                roundtrip_unstable.append(c.name)
        except Exception:
            roundtrip_unstable.append(c.name)
    if has_json and parsed:
        raw = (c / "in.json").read_text().lstrip()
        try:
            # An empty in.json means the stream yields no document, which loads
            # as None (comment-only / empty inputs).
            expected = None if not raw else json.JSONDecoder().raw_decode(raw)[0]
            if not jequal(val, expected):
                json_mismatch.append(c.name)
        except Exception:
            json_mismatch.append(c.name)

out = {
    "_comment": "Behavior baseline for the yaml-test-suite submodule. YAMLRocks is a "
    "pragmatic YAML 1.2 parser; these lists record cases it does not yet "
    "handle so the suite guards against regressions. Shrinking these lists "
    "is an improvement (regenerate with "
    "tests/data/yaml_test_suite/generate_expectations.py).",
    "roundtrip_unstable": sorted(roundtrip_unstable),
    "json_mismatch": sorted(json_mismatch),
    "rejected": sorted(rejected),
    "error_accepted": sorted(error_accepted),
    "parse_failures": sorted(parse_failures),
}
pathlib.Path("tests/data/yaml_test_suite/expectations.json").write_text(
    json.dumps(out, indent=2) + "\n"
)
print(
    f"roundtrip_unstable={len(roundtrip_unstable)} json_mismatch={len(json_mismatch)} "
    f"rejected={len(rejected)} error_accepted={len(error_accepted)} "
    f"parse_failures={len(parse_failures)}"
)
