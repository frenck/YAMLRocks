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
# Every directory with an ``in.yaml`` is a case, including the variants stored
# under numbered subdirectories (``DE56/00``, ...); the id is the path relative
# to the suite root, matching the test harness.
for c in sorted((p.parent for p in base.rglob("in.yaml")), key=lambda p: p.as_posix()):
    cid = c.relative_to(base).as_posix()
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
        rejected.append(cid)
        continue
    if is_err and not parsed:
        # An invalid document with an in.json (showing a lenient parse) that we
        # still reject. Correct behavior, tracked like any other rejection.
        rejected.append(cid)
        continue
    if is_err and parsed:
        # The suite marks this input invalid, but the pragmatic parser accepts
        # it. Track the laxness so it cannot grow silently and can only shrink.
        error_accepted.append(cid)
    if not is_err and not parsed:
        # A valid document the pragmatic parser does not handle yet. Baselined
        # so the gap is visible and can only shrink as the parser improves.
        parse_failures.append(cid)
    if parsed:
        # Round-trip must be byte-for-byte identical for an unmodified document.
        try:
            emitted = yamlrocks.loads(inp, option=yamlrocks.OPT_ROUND_TRIP).to_yaml()
            if emitted != inp:
                roundtrip_unstable.append(cid)
        except Exception:
            roundtrip_unstable.append(cid)
    if has_json and parsed and not is_err:
        # Only valid cases are compared against their canonical JSON. An error
        # case's answer is "reject", so a lenient accept is governed by
        # error_accepted, not by matching the JSON a lenient parser would yield.
        raw = (c / "in.json").read_text().lstrip()
        try:
            # An empty in.json means the stream yields no document, which loads
            # as None (comment-only / empty inputs).
            expected = None if not raw else json.JSONDecoder().raw_decode(raw)[0]
            if not jequal(val, expected):
                json_mismatch.append(cid)
        except Exception:
            json_mismatch.append(cid)

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
