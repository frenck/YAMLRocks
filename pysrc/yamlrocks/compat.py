"""PyYAML-compatible API shim.

A drop-in replacement for the most common PyYAML entry points, so existing code
can switch to YAMLRocks with a one-line import change::

    # before
    import yaml
    # after
    import yamlrocks.compat as yaml

    data = yaml.safe_load(text)
    text = yaml.safe_dump(data)

Only the safe subset of PyYAML is mirrored. YAMLRocks does not execute arbitrary
Python tags, so ``load``/``dump`` behave like ``safe_load``/``safe_dump``.
"""

from __future__ import annotations

from typing import IO, Any

import yamlrocks

__all__ = [
    "YAMLError",
    "dump",
    "dump_all",
    "load",
    "load_all",
    "safe_dump",
    "safe_dump_all",
    "safe_load",
    "safe_load_all",
]

# PyYAML raises yaml.YAMLError (a subclass of Exception) for both reading and
# writing. Alias it to YAMLRocks's common base so ``except yaml.YAMLError`` keeps
# catching either side.
YAMLError = yamlrocks.YAMLRocksError


def _read(stream: Any) -> Any:
    """Accept a str/bytes document or a file-like object, like PyYAML."""
    if hasattr(stream, "read"):
        return stream.read()
    return stream


def safe_load(stream: Any) -> Any:
    """Parse the first YAML document from ``stream`` (str, bytes, or file)."""
    return yamlrocks.loads(_read(stream))


def safe_load_all(stream: Any) -> list[Any]:
    """Parse all YAML documents from ``stream``.

    PyYAML returns a generator; this returns a list, which is iterable in the
    same way for the common ``list(yaml.safe_load_all(...))`` usage.
    """
    return yamlrocks.loads_all(_read(stream))


# YAMLRocks does not deserialize arbitrary Python objects, so the full-power
# ``load``/``load_all`` map onto the safe variants. The ``Loader`` argument is
# accepted and ignored for signature compatibility.
def load(stream: Any, Loader: Any = None) -> Any:
    """Parse the first document from ``stream``; an alias for :func:`safe_load`."""
    return safe_load(stream)


def load_all(stream: Any, Loader: Any = None) -> list[Any]:
    """Parse all documents from ``stream``; an alias for :func:`safe_load_all`."""
    return safe_load_all(stream)


def _dump_options(default_flow_style: bool, sort_keys: bool, indent: int | None) -> int:
    option = 0
    if default_flow_style:
        option |= yamlrocks.OPT_FLOW_STYLE
    if sort_keys:
        option |= yamlrocks.OPT_SORT_KEYS
    # YAMLRocks offers two block-indent widths, 2 and 4. PyYAML accepts any
    # width; we map an explicit 4 to four spaces and everything else (including
    # the ``None`` default) to two, matching PyYAML's own default of 2.
    if indent == 4:
        option |= yamlrocks.OPT_INDENT_4
    else:
        option |= yamlrocks.OPT_INDENT_2
    return option


def safe_dump(
    data: Any,
    stream: IO[str] | None = None,
    *,
    default_flow_style: bool = False,
    sort_keys: bool = True,
    indent: int | None = None,
    width: int | None = None,
    **_ignored: Any,
) -> str | None:
    """Serialize ``data`` to YAML.

    Returns a ``str`` when ``stream`` is ``None`` (PyYAML semantics), otherwise
    writes to ``stream`` and returns ``None``. Extra PyYAML keyword arguments are
    accepted and ignored.

    ``width`` is honored as a best-effort line-wrap limit. Unlike PyYAML (which
    wraps at 80 by default), the default here leaves lines unwrapped; pass an
    explicit ``width`` to wrap.
    """
    option = _dump_options(default_flow_style, sort_keys, indent)
    text = yamlrocks.dumps(data, option=option, width=width).decode("utf-8")
    if stream is None:
        return text
    stream.write(text)
    return None


def safe_dump_all(
    documents: Any,
    stream: IO[str] | None = None,
    **kwargs: Any,
) -> str | None:
    """Serialize a sequence of documents, separated by ``---``."""
    parts: list[str] = [safe_dump(doc, **kwargs) or "" for doc in documents]
    text = "---\n".join(parts)
    if stream is None:
        return text
    stream.write(text)
    return None


# ``dump`` is the safe dumper here (YAMLRocks never emits Python-specific tags).
def dump(
    data: Any,
    stream: IO[str] | None = None,
    Dumper: Any = None,
    **kwargs: Any,
) -> str | None:
    """Serialize ``data`` to YAML; an alias for :func:`safe_dump`."""
    return safe_dump(data, stream, **kwargs)


def dump_all(
    documents: Any,
    stream: IO[str] | None = None,
    Dumper: Any = None,
    **kwargs: Any,
) -> str | None:
    """Serialize a sequence of documents; an alias for :func:`safe_dump_all`."""
    return safe_dump_all(documents, stream, **kwargs)
