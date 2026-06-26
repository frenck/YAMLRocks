"""Security regression tests.

Covers the historical PyYAML/ruamel.yaml vulnerability classes and the
denial-of-service vectors common to YAML parsers, ensuring YAMLRocks is not
affected:

* arbitrary code execution via ``!!python/object`` tags
  (CVE-2017-18342, CVE-2020-1747, CVE-2020-14343);
* "billion laughs" alias-expansion memory bombs;
* stack exhaustion from deeply nested input;
* infinite recursion from circular includes;
* path/argument handling for the include and secret tags.

Every test runs under the suite-wide memory cap from ``conftest.py``.
"""

from __future__ import annotations

import sys

import pytest

import yamlrocks

# -- Arbitrary code execution (the PyYAML RCE CVEs) --------------------------

RCE_PAYLOADS = [
    b"!!python/object/apply:os.system ['echo pwned']",
    b"!!python/object/apply:subprocess.check_output [['id']]",
    b"!!python/object/new:type [str, [], {}]",
    b"!!python/name:os.system",
    b"x: !!python/object/apply:os.system ['touch /tmp/yamlrocks_pwned']",
]


@pytest.mark.parametrize("payload", RCE_PAYLOADS)
def test_python_object_tags_do_not_execute(payload, tmp_path, monkeypatch):
    """`!!python/object` tags must never construct or call Python objects."""
    called = []
    monkeypatch.setattr("os.system", lambda *a, **k: called.append(a))
    # YAMLRocks has no mechanism to build arbitrary objects; it returns inert data
    # (or raises), and crucially never invokes os.system.
    try:
        yamlrocks.loads(payload)
    except yamlrocks.YAMLRocksDecodeError:
        pass
    assert called == []


def test_python_object_value_is_inert_data():
    """A python/object payload is returned as plain data, not an object."""
    result = yamlrocks.loads(b"!!python/object/apply:os.system ['echo hi']")
    assert result == ["echo hi"]


# -- Billion laughs (alias expansion) ----------------------------------------


def _alias_bomb(width: int, depth: int) -> bytes:
    lines = ["a0: &a0 [" + ", ".join(["x"] * width) + "]"]
    for i in range(1, depth):
        refs = ", ".join([f"*a{i - 1}"] * width)
        lines.append(f"a{i}: &a{i} [{refs}]")
    return ("\n".join(lines) + "\n").encode()


def test_billion_laughs_is_rejected():
    """An exponential alias bomb is rejected instead of exhausting memory."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match=r"too many nodes|alias"):
        yamlrocks.loads(_alias_bomb(width=10, depth=10))


def test_modest_aliases_still_work():
    """Reasonable, non-pathological alias use is unaffected by the guard."""
    src = b"base: &b {x: 1, y: 2}\nuse1: *b\nuse2: *b\nuse3: *b\n"
    result = yamlrocks.loads(src)
    assert result["use1"] == {"x": 1, "y": 2}
    assert result["use3"] == {"x": 1, "y": 2}


# -- Deep nesting (stack exhaustion) -----------------------------------------


@pytest.mark.parametrize("opener", [b"[", b"{"])
def test_deeply_nested_flow_is_rejected(opener):
    """Deeply nested flow collections hit the depth limit, not a crash."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="nesting depth"):
        yamlrocks.loads(opener * 5000)


def test_deeply_nested_block_is_bounded():
    """Deeply nested block mappings are handled without crashing."""
    src = "".join(f"{'  ' * i}k{i}:\n" for i in range(2000))
    # Either parses or raises a clean depth error, but never crashes.
    try:
        yamlrocks.loads(src.encode())
    except yamlrocks.YAMLRocksDecodeError:
        pass


def test_reasonable_nesting_still_parses():
    """Nesting well within the limit parses correctly."""
    depth = 100
    src = "".join(f"{'  ' * i}k{i}:\n" for i in range(depth)) + "  " * depth + "v: 1\n"
    result = yamlrocks.loads(src.encode())
    assert isinstance(result, dict)


# Run in a subprocess so that a regression (a real stack overflow aborts the
# interpreter) fails this test cleanly instead of killing the whole pytest run.
# A 1 MB worker stack at near-maximum nesting overflowed the recursive Rust paths
# before the ``stacker`` integration grew the native stack on demand.
_SMALL_STACK_SCRIPT = """
import threading, yamlrocks
deep_map = b"".join(b" " * i + b"k:\\n" for i in range(%(depth)d))
deep_seq = b"[" * %(depth)d + b"]" * %(depth)d
py_deep = None
for _ in range(%(depth)d):
    py_deep = {"k": py_deep}
def work():
    yamlrocks.loads(deep_map)            # fast decode + value-tree teardown
    yamlrocks.loads(deep_seq)            # deep flow
    doc = yamlrocks.loads(deep_map, option=yamlrocks.OPT_ROUND_TRIP)
    doc.to_yaml()                        # compose + comment walk + emit
    doc.to_dict()                        # AST -> Python conversion
    yamlrocks.dumps(py_deep)             # Python -> value tree + emit
    yamlrocks.to_json(py_deep)
threading.stack_size(1024 * 1024)
t = threading.Thread(target=work)
t.start()
t.join()
print("OK")
"""


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX thread stack sizing")
def test_deeply_nested_input_survives_small_thread_stack():
    """Deeply nested input must not overflow a worker thread's stack and abort.

    The recursive scan/decode/compose/comment-attach/emit/teardown paths grow the
    native stack on demand (the ``stacker`` integration), so a 1 MB worker thread
    parsing and emitting near-maximum-depth input succeeds cleanly instead of
    segfaulting the interpreter under ``panic = "abort"`` (it overflowed around
    depth 400 before). Runs in a subprocess so a regression cannot abort pytest.
    """
    import subprocess

    script = _SMALL_STACK_SCRIPT % {"depth": 900}
    proc = subprocess.run(
        [sys.executable, "-c", script], capture_output=True, timeout=60
    )
    assert proc.returncode == 0, (
        f"deep input crashed on a 1 MB thread stack: rc={proc.returncode} "
        f"stderr={proc.stderr.decode()[:400]}"
    )
    assert proc.stdout.strip() == b"OK"


# -- Circular includes -------------------------------------------------------


def test_circular_include_is_rejected(tmp_path):
    """A cycle a -> b -> a must raise, not recurse forever."""
    (tmp_path / "a.yaml").write_text("x: !include b.yaml\n")
    (tmp_path / "b.yaml").write_text("y: !include a.yaml\n")
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="circular include"):
        yamlrocks.loads(
            b"root: !include a.yaml\n",
            option=yamlrocks.OPT_INCLUDES,
            include_dir=str(tmp_path),
        )


def test_self_include_is_rejected(tmp_path):
    """A file including itself is a cycle and must be rejected."""
    (tmp_path / "self.yaml").write_text("x: !include self.yaml\n")
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="circular include"):
        yamlrocks.loads(
            b"r: !include self.yaml\n",
            option=yamlrocks.OPT_INCLUDES,
            include_dir=str(tmp_path),
        )


def test_include_chain_depth_is_bounded(tmp_path):
    """A long acyclic include chain is rejected before exhausting the stack."""
    for i in range(60):
        target = f"!include f{i + 1}.yaml" if i < 59 else "done"
        (tmp_path / f"f{i}.yaml").write_text(f"v: {target}\n")
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="depth"):
        yamlrocks.load(str(tmp_path / "f0.yaml"), option=yamlrocks.OPT_INCLUDES)


# -- Include path confinement ------------------------------------------------


def test_include_rejects_parent_traversal(tmp_path):
    """An include escaping the base directory via ``..`` is rejected."""
    (tmp_path / "outside.yaml").write_text("secret: data\n")
    cfg = tmp_path / "config"
    cfg.mkdir()
    (cfg / "main.yaml").write_text("x: !include ../outside.yaml\n")
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="outside"):
        yamlrocks.load(str(cfg / "main.yaml"), option=yamlrocks.OPT_INCLUDES)


def test_include_rejects_absolute_path(tmp_path):
    """An absolute include target outside the base directory is rejected."""
    outside = tmp_path / "outside.yaml"
    outside.write_text("secret: data\n")
    cfg = tmp_path / "config"
    cfg.mkdir()
    (cfg / "main.yaml").write_text(f"x: !include {outside}\n")
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="outside"):
        yamlrocks.load(str(cfg / "main.yaml"), option=yamlrocks.OPT_INCLUDES)


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX symlink semantics")
def test_include_dir_rejects_symlink_escape(tmp_path):
    """A symlink inside an ``!include_dir_*`` directory cannot escape the base."""
    secret = tmp_path / "outside.yaml"
    secret.write_text("password: hunter2\n")
    cfg = tmp_path / "config"
    cfg.mkdir()
    pkgs = cfg / "packages"
    pkgs.mkdir()
    # A planted symlink resolving outside the base tree.
    (pkgs / "leak.yaml").symlink_to(secret)
    (cfg / "main.yaml").write_text("data: !include_dir_named packages\n")
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="outside"):
        yamlrocks.load(str(cfg / "main.yaml"), option=yamlrocks.OPT_INCLUDES)


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX symlink semantics")
def test_include_dir_list_rejects_symlink_escape(tmp_path):
    """``!include_dir_list`` confines its entries the same way."""
    secret = tmp_path / "outside.yaml"
    secret.write_text("- leaked\n")
    cfg = tmp_path / "config"
    cfg.mkdir()
    items = cfg / "items"
    items.mkdir()
    (items / "leak.yaml").symlink_to(secret)
    (cfg / "main.yaml").write_text("data: !include_dir_list items\n")
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="outside"):
        yamlrocks.load(str(cfg / "main.yaml"), option=yamlrocks.OPT_INCLUDES)


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX symlink semantics")
def test_include_dangling_symlink_is_not_read(tmp_path):
    """A `!include` of a dangling symlink is refused, not read through.

    A symlink whose target does not currently resolve cannot be confined: it
    would have to be re-followed at read time, and the target could be swapped to
    point outside the base after the confinement check (a TOCTOU escape). Such a
    symlink is reported as not-found rather than opened.
    """
    outside = tmp_path / "outside.yaml"
    outside.write_text("leaked: true\n")
    cfg = tmp_path / "config"
    cfg.mkdir()
    # A symlink to a (currently) non-existent in-base target: it does not
    # canonicalize, so it must not be returned for reading.
    (cfg / "link.yaml").symlink_to(cfg / "absent.yaml")
    (cfg / "main.yaml").write_text("x: !include link.yaml\n")
    with pytest.raises(yamlrocks.YAMLRocksDecodeError):
        yamlrocks.load(str(cfg / "main.yaml"), option=yamlrocks.OPT_INCLUDES)


def test_include_dir_missing_directory_resolves_to_empty(tmp_path):
    """A `!include_dir_*` of a genuinely-missing (non-symlink) directory yields an
    empty mapping, matching Home Assistant, rather than erroring."""
    cfg = tmp_path / "config"
    cfg.mkdir()
    # `themes/` does not exist, but the path stays inside the base directory.
    (cfg / "main.yaml").write_text("themes: !include_dir_merge_named themes\n")
    data = yamlrocks.load(str(cfg / "main.yaml"), option=yamlrocks.OPT_INCLUDES)
    assert data == {"themes": {}}


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX symlink semantics")
def test_include_dir_entry_read_through_canonical_path(tmp_path):
    """An in-base directory entry is read through its canonical (symlink-resolved)
    path, so the read targets the validated file rather than the pre-check
    candidate, while the entry is still keyed by its own filename."""
    cfg = tmp_path / "config"
    cfg.mkdir()
    pkgs = cfg / "packages"
    pkgs.mkdir()
    (pkgs / "real.yaml").write_text("value: 42\n")
    # An in-base symlink is allowed and followed to its real target on read.
    (pkgs / "alias.yaml").symlink_to(pkgs / "real.yaml")
    (cfg / "main.yaml").write_text("data: !include_dir_named packages\n")
    data = yamlrocks.load(str(cfg / "main.yaml"), option=yamlrocks.OPT_INCLUDES)
    # Keyed by the entry's own name (`alias`), not the resolved target (`real`),
    # and the canonical read returns the target's content.
    assert data["data"]["alias"] == {"value": 42}
    assert data["data"]["real"] == {"value": 42}


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX symlink semantics")
def test_secrets_symlink_escape_is_not_read(tmp_path):
    """A ``secrets.yaml`` symlinked outside the base tree is never read."""
    outside = tmp_path / "real_secrets.yaml"
    outside.write_text("api_key: leaked\n")
    cfg = tmp_path / "config"
    cfg.mkdir()
    # secrets.yaml points outside the base, so the secret stays unresolved.
    (cfg / "secrets.yaml").symlink_to(outside)
    (cfg / "main.yaml").write_text("token: !secret api_key\n")
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="not defined"):
        yamlrocks.load(
            str(cfg / "main.yaml"),
            option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_SECRETS,
        )


# -- Environment exposure via !env_var ---------------------------------------


def test_env_var_reaches_arbitrary_environment_when_enabled(monkeypatch):
    """``OPT_ENV_VAR`` grants the document any environment variable by name.

    This documents the trust boundary: there is no allowlist, so the flag must
    only be enabled for configuration the caller fully trusts. See the security
    reference.
    """
    monkeypatch.setenv("YAMLROCKS_FAKE_SECRET", "leaked")
    data = yamlrocks.loads(
        b"value: !env_var YAMLROCKS_FAKE_SECRET\n",
        option=yamlrocks.OPT_ENV_VAR,
    )
    assert data["value"] == "leaked"


def test_env_var_is_inert_without_the_flag(monkeypatch):
    """Without ``OPT_ENV_VAR`` the environment is never consulted."""
    monkeypatch.setenv("YAMLROCKS_FAKE_SECRET", "leaked")
    data = yamlrocks.loads(b"value: !env_var YAMLROCKS_FAKE_SECRET\n")
    # The tag is inert: the scalar passes through literally, never expanded.
    assert data["value"] == "YAMLROCKS_FAKE_SECRET"


# -- Encoder callback reentrancy ---------------------------------------------


def test_dumps_default_mutating_outer_list_is_memory_safe():
    """A ``default`` callback that mutates the list being dumped must not crash.

    The encoder snapshots an exact list before conversion, so a callback that
    clears it mid-dump operates on a stable snapshot rather than a freed buffer.
    """

    class Unsupported:
        pass

    data = [1, Unsupported(), 3]

    def default(_obj):
        data.clear()  # mutate the very list being walked
        return "x"

    out = yamlrocks.dumps(data, default=default)
    assert yamlrocks.loads(out) == [1, "x", 3]


def test_dumps_default_mutating_outer_dict_is_memory_safe():
    """The same protection for an exact dict walked while ``default`` runs."""

    class Unsupported:
        pass

    data = {"a": 1, "b": Unsupported(), "c": 3}

    def default(_obj):
        data.clear()  # `PyDict_Next` forbids mutation mid-walk; we snapshot first
        return "x"

    out = yamlrocks.dumps(data, default=default)
    assert yamlrocks.loads(out) == {"a": 1, "b": "x", "c": 3}


# -- Malformed input ---------------------------------------------------------


def test_invalid_utf8_is_rejected_cleanly():
    """Invalid UTF-8 raises a clean error rather than crashing.

    Leading ASCII rules out UTF-16/32 BOM detection, so the bytes are decoded as
    UTF-8 and the malformed sequence is rejected.
    """
    with pytest.raises(ValueError):
        yamlrocks.loads(b"a: \xc0\x80bad")


@pytest.mark.parametrize(
    "junk",
    [b"\x00\x01\x02", b"{[}]", b": : :", b"!!!!", b"&&&&", b"****", b"]]]]"],
)
def test_random_junk_does_not_crash(junk):
    """Structurally invalid input returns or raises, never crashes."""
    try:
        yamlrocks.loads(junk)
    except (yamlrocks.YAMLRocksDecodeError, ValueError, TypeError):
        pass


def test_large_flat_document_is_handled():
    """A large but flat document parses without issue."""
    src = b"".join(b"key%d: value%d\n" % (i, i) for i in range(50000))
    result = yamlrocks.loads(src)
    assert len(result) == 50000


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX path semantics")
def test_missing_include_does_not_leak_paths(tmp_path):
    """A missing include raises a decode error, not a raw OS error."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError):
        yamlrocks.loads(
            b"x: !include /nonexistent/secret.yaml\n",
            option=yamlrocks.OPT_INCLUDES,
            include_dir=str(tmp_path),
        )


def test_dumps_mutating_container_does_not_use_after_free():
    """Converting an element runs Python (here a fake datetime mutates the list);
    dumps must snapshot first, not walk freed storage (was a SIGSEGV)."""
    import gc

    lst = []

    class FakeDate:
        @property
        def isoformat(self):
            def f():
                del lst[:]
                gc.collect()
                return "2020-01-01"

            return f

    lst += [[i] * 8 for i in range(60)]
    lst.append(FakeDate())
    lst += [[i] * 8 for i in range(60)]
    # Must not crash (it raises or succeeds, but the process survives).
    try:
        yamlrocks.dumps(lst)
    except Exception:
        pass


def test_roundtrip_assign_mutating_dict_does_not_panic():
    """Assigning a dict whose conversion mutates it must not panic the process.

    Building the AST node runs Python (here a value's ``__float__`` adds a key);
    the round-trip converter snapshots the entries first, so it does not iterate
    the live dict (which raised ``dictionary changed size during iteration``).
    """
    doc = yamlrocks.loads(b"x: 1\n", option=yamlrocks.OPT_ROUND_TRIP)

    class Evil:
        def __init__(self, d):
            self.d = d

        def __float__(self):
            self.d["injected"] = 99  # mutate the dict mid-conversion
            return 1.0

    data = {"a": None, "b": None}
    data["a"] = Evil(data)
    doc["x"] = data  # must not crash
    assert b"a: 1.0" in yamlrocks.dumps(doc)


def test_roundtrip_assign_mutating_list_does_not_panic():
    """The same protection for a list assigned into a round-trip document."""
    doc = yamlrocks.loads(b"x: 1\n", option=yamlrocks.OPT_ROUND_TRIP)
    items: list = []

    class Evil:
        def __float__(self):
            items.append("injected")  # mutate the list mid-conversion
            return 2.0

    items.extend([Evil(), None])
    doc["x"] = items  # must not crash
    yamlrocks.dumps(doc)


def test_annotated_alias_bomb_is_bounded():
    """A doubling alias bomb (2^N nodes at N hops) must be bounded on the
    annotated/round-trip paths, not expand until OOM."""
    src = "a0: &a0 [x]\n"
    for i in range(1, 40):
        src += f"a{i}: &a{i} [*a{i - 1}, *a{i - 1}]\n"
    src += "top: *a39\n"
    # Returns quickly and bounded, rather than hanging or exhausting memory.
    doc = yamlrocks.loads(src.encode(), option=yamlrocks.OPT_ROUND_TRIP)
    assert doc.to_yaml()  # terminates


def test_schema_validation_alias_bomb_is_bounded():
    """Validating a round-trip document against a schema must bound alias
    expansion, not abort the process.

    The schema validator has its own alias-expansion pass; before it shared the
    node budget the rest of the library uses, a sub-400-byte bomb loaded with
    OPT_ROUND_TRIP plus any schema expanded exponentially and aborted the
    interpreter (an uncatchable SIGABRT), bypassing the decode-time guard.
    """
    src = "a0: &a0 [x]\n"
    for i in range(1, 40):
        src += f"a{i}: &a{i} [*a{i - 1}, *a{i - 1}]\n"
    src += "top: *a39\n"
    # Must terminate (raise or succeed) without aborting the process.
    try:
        yamlrocks.loads(
            src.encode(), option=yamlrocks.OPT_ROUND_TRIP, schema={"type": "object"}
        )
    except Exception:
        pass


def test_schema_validation_resolves_aliases():
    """The schema validator still sees an alias as its referent, not null."""
    src = b"defaults: &d {port: 80}\nserver: *d\n"
    schema = {
        "type": "object",
        "properties": {
            "server": {
                "type": "object",
                "properties": {"port": {"type": "integer"}},
                "required": ["port"],
            }
        },
    }
    # Validates cleanly: `*d` resolves to `{port: 80}`, satisfying the schema.
    assert yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP, schema=schema)
