"""Async wrappers: run the sync core off the event loop thread.

These call the same parser/emitter as the sync API, but in a worker thread, so
an asyncio application does not block its loop. The native core releases the GIL
for the heavy phases, which the loop-not-blocked test exercises directly.

``asyncio_mode = "auto"`` (see pyproject.toml) runs every ``async def test_*``
as a coroutine, so no per-test marker is needed.
"""

from __future__ import annotations

import asyncio

import yamlrocks


def test_no_async_serializers():
    """There are deliberately no async serializers: serializing holds the GIL
    for the object traversal, so a thread offload buys little. Async is reserved
    for loading and the file APIs."""
    assert not hasattr(yamlrocks, "async_dumps")
    assert not hasattr(yamlrocks, "async_to_json")


def test_import_does_not_pull_in_asyncio():
    """`import yamlrocks` must not import asyncio: it is imported lazily inside the
    async_* helpers, so the sync import path stays cheap (issue #52)."""
    import subprocess
    import sys

    code = "import sys, yamlrocks; sys.exit(1 if 'asyncio' in sys.modules else 0)"
    result = subprocess.run(
        [sys.executable, "-c", code], capture_output=True, text=True
    )
    assert result.returncode == 0, (
        f"importing yamlrocks pulled in asyncio; keep it lazy.\n{result.stderr}"
    )


async def test_async_loads_matches_loads():
    """async_loads returns the same result as loads."""
    src = b"name: app\nport: 8080\nlist:\n  - 1\n  - 2\n"
    assert await yamlrocks.async_loads(src) == yamlrocks.loads(src)


async def test_async_loads_options():
    """async_loads honors option flags."""
    out = await yamlrocks.async_loads(b"a: yes", option=yamlrocks.OPT_YAML_1_1)
    assert out == {"a": True}


async def test_async_loads_tags():
    """async_loads honors the tag registry."""
    out = await yamlrocks.async_loads(b"p: !vec [1, 2]", tags={"!vec": tuple})
    assert out == {"p": (1, 2)}


async def test_async_load_reads_file(tmp_path):
    """async_load reads and parses a file off the loop thread."""
    p = tmp_path / "c.yaml"
    p.write_text("name: app\nport: 8080\n")
    assert await yamlrocks.async_load(str(p)) == {"name": "app", "port": 8080}


async def test_async_load_all_reads_multidoc(tmp_path):
    """async_load_all returns every document from a file."""
    p = tmp_path / "m.yaml"
    p.write_text("---\na: 1\n---\nb: 2\n")
    assert await yamlrocks.async_load_all(str(p)) == [{"a": 1}, {"b": 2}]


async def test_async_loads_all_parses_multidoc_stream():
    """async_loads_all returns every document from an in-memory stream."""
    src = b"---\na: 1\n---\nb: 2\n"
    assert await yamlrocks.async_loads_all(src) == yamlrocks.loads_all(src)


async def test_async_dump_writes_file(tmp_path):
    """async_dump serializes and writes to a path."""
    p = tmp_path / "out.yaml"
    await yamlrocks.async_dump({"name": "app"}, str(p))
    assert p.read_bytes() == b"name: app\n"


async def test_async_load_with_includes(tmp_path):
    """async_load resolves includes, defaulting include_dir to the file directory."""
    (tmp_path / "main.yaml").write_text("data: !include child.yaml\n")
    (tmp_path / "child.yaml").write_text("key: value\n")
    out = await yamlrocks.async_load(
        str(tmp_path / "main.yaml"), option=yamlrocks.OPT_INCLUDES
    )
    assert out == {"data": {"key": "value"}}


async def test_async_round_trip_document(tmp_path):
    """async_load returns a usable round-trip YAMLRocksDocument."""
    p = tmp_path / "c.yaml"
    p.write_text("# comment\nname: app\n")
    doc = await yamlrocks.async_load(str(p), option=yamlrocks.OPT_ROUND_TRIP)
    assert b"# comment" in doc.to_yaml()


async def test_loop_not_blocked_during_parse():
    """The GIL is released during the parse, so other tasks keep running.

    A background ticker sleeps in small steps while a large document parses in a
    worker thread. If the parse held the GIL, the ticker could not advance.
    """
    ticks = 0

    async def ticker():
        nonlocal ticks
        for _ in range(5):
            await asyncio.sleep(0.001)
            ticks += 1

    big = b"items:\n" + b"".join(b"  - k%d: %d\n" % (i, i) for i in range(50000))
    result, _ = await asyncio.gather(yamlrocks.async_loads(big), ticker())
    assert ticks == 5  # the loop kept ticking throughout the parse
    assert len(result["items"]) == 50000


async def test_async_loads_concurrent():
    """Several async_loads calls can be gathered and all complete."""
    docs = [b"a: %d" % i for i in range(10)]
    results = await asyncio.gather(*(yamlrocks.async_loads(d) for d in docs))
    assert results == [{"a": i} for i in range(10)]
