"""YAMLRocks: a fast, correct YAML library for Python."""

from __future__ import annotations

import contextlib
import os
from collections.abc import Callable, Iterator
from typing import Any

from yamlrocks._yamlrocks import (
    OPT_ANNOTATE_NUMBERS,
    OPT_ANNOTATED,
    OPT_DUPLICATE_KEYS_ERROR,
    OPT_DUPLICATE_KEYS_WARN,
    OPT_ENV_VAR,
    OPT_ENV_VAR_NOT_FOUND_WARN,
    OPT_EXPLICIT_END,
    OPT_EXPLICIT_START,
    OPT_FLOW_STYLE,
    OPT_INCLUDE_DIR_RECURSIVE,
    OPT_INCLUDES,
    OPT_INDENT_2,
    OPT_INDENT_4,
    OPT_INDENTLESS_SEQUENCES,
    OPT_NAIVE_UTC,
    OPT_NULL_AS_KEYWORD,
    OPT_NULL_AS_TILDE,
    OPT_OMIT_MICROSECONDS,
    OPT_PASSTHROUGH_DATACLASS,
    OPT_PASSTHROUGH_DATETIME,
    OPT_PASSTHROUGH_TAG,
    OPT_PYYAML_COMPAT,
    OPT_REJECT_COMPLEX_KEYS,
    OPT_ROUND_TRIP,
    OPT_SECRET_NOT_FOUND_WARN,
    OPT_SECRETS,
    OPT_SERIALIZE_NUMPY,
    OPT_SINGLE_QUOTES,
    OPT_SORT_KEYS,
    OPT_TIMESTAMPS,
    OPT_UPGRADE_1_1,
    OPT_UTC_Z,
    OPT_YAML_1_1,
    OPT_YAML_1_1_WARN,
    YAMLRocksAnnotatedDict,
    YAMLRocksAnnotatedList,
    YAMLRocksDocument,
    YAMLRocksDocumentView,
    YAMLRocksMapping,
    YAMLRocksNode,
    YAMLRocksScalar,
    YAMLRocksSequence,
    YAMLRocksTag,
    dump_includes,
    dump_includes_map,
    dumps,
    loads,
    loads_all,
    schema_ref,
    to_json,
    yaml_version,
)
from yamlrocks.exceptions import (
    YAMLRocksCircularIncludeError,
    YAMLRocksComplexKeyError,
    YAMLRocksDecodeError,
    YAMLRocksDuplicateKeyError,
    YAMLRocksEncodeError,
    YAMLRocksEnvVarError,
    YAMLRocksError,
    YAMLRocksIncludeConfinementError,
    YAMLRocksIncludeDepthError,
    YAMLRocksIncludeError,
    YAMLRocksIncludeNotFoundError,
    YAMLRocksParseError,
    YAMLRocksSchemaError,
    YAMLRocksSecretError,
    YAMLRocksSecretNotFoundError,
    YAMLRocksUnserializableError,
)


class YAMLRocksTags(dict):
    """A registry mapping custom YAML tags to handler functions.

    A ``YAMLRocksTags`` instance is an ordinary ``dict`` of ``{tag: function}`` where each
    function is called with the tag's already-resolved inner value and returns
    the object to place in the result. Because it is just a ``dict``, you can
    also pass a plain ``{tag: function}`` mapping to ``loads``/``load`` directly;
    ``YAMLRocksTags`` only adds the :meth:`register` decorator for convenience.

    YAMLRocksTags registered here take precedence over a ``tag_handler`` catch-all, which
    still handles any tag not in the registry.

    Example::

        tags = yamlrocks.YAMLRocksTags()

        @tags.register("!vec")
        def _(value):
            return tuple(value)

        yamlrocks.loads(b"point: !vec [1, 2]", tags=tags)
        # {'point': (1, 2)}
    """

    def register(
        self,
        tag: str,
        func: Callable[[Any], Any] | None = None,
    ) -> Callable[[Any], Any]:
        """Register ``func`` for ``tag``; usable directly or as a decorator.

        Called with two arguments it registers immediately and returns ``func``.
        Called with just a tag it returns a decorator, so it can be used as
        ``@tags.register("!tag")`` above a function definition.
        """
        if func is None:

            def decorator(f: Callable[[Any], Any]) -> Callable[[Any], Any]:
                self[tag] = f
                return f

            return decorator

        self[tag] = func
        return func


# The five built-in `!include` family directives, for the ``is_include`` predicate.
_INCLUDE_TAGS = frozenset(
    {
        "!include",
        "!include_dir_list",
        "!include_dir_named",
        "!include_dir_merge_list",
        "!include_dir_merge_named",
    }
)


class _SourceTagProvenance:
    """Mixin adding the ``is_secret``/``is_env_var``/``is_include`` predicates over
    ``__source_tag__``, shared by the annotated scalar subclasses.

    ``__source_tag__`` is the config/custom tag that produced the node (``!secret``,
    ``!env_var``, an ``!include*`` directive, or a custom ``!mytag``), or ``None``.
    The booleans are convenience checks over the built-in config-tag subset.
    """

    __slots__ = ()

    __source_tag__: str | None

    @property
    def is_secret(self) -> bool:
        """Whether this value was produced by a ``!secret`` directive."""
        return self.__source_tag__ == "!secret"

    @property
    def is_env_var(self) -> bool:
        """Whether this value was produced by an ``!env_var`` directive."""
        return self.__source_tag__ == "!env_var"

    @property
    def is_include(self) -> bool:
        """Whether this value was produced by any ``!include`` directive."""
        return self.__source_tag__ in _INCLUDE_TAGS


class YAMLRocksAnnotatedStr(str, _SourceTagProvenance):
    """A ``str`` subclass carrying source-location metadata.

    Returned for string scalars in annotated mode (``OPT_ANNOTATED``), mirroring
    Home Assistant's ``NodeStrClass``. It behaves exactly like ``str`` while also
    exposing ``__line__``, ``__column__``, ``__file__``, the end position
    ``__end_line__`` / ``__end_column__`` (just past the last character, like
    PyYAML's ``end_mark``), ``__style__`` (the source scalar style: one of
    ``"plain"``, ``"single"``, ``"double"``, ``"literal"`` for ``|``, or
    ``"folded"`` for ``>``), and ``__source_tag__`` (the config/custom tag that
    produced it, with the ``is_secret``/``is_env_var``/``is_include`` predicates).

    Defined in Python because a native extension cannot efficiently subclass the
    immutable ``str`` type.
    """

    __slots__ = (
        "__column__",
        "__end_column__",
        "__end_line__",
        "__end_offset__",
        "__file__",
        "__line__",
        "__offset__",
        "__source_tag__",
        "__source_target__",
        "__style__",
    )

    __line__: int
    __column__: int
    __file__: str | None
    __end_line__: int
    __end_column__: int
    __offset__: int
    __end_offset__: int
    __style__: str

    def __new__(
        cls,
        value: str,
        line: int = 0,
        column: int = 0,
        config_file: str | None = None,
        end_line: int = 0,
        end_column: int = 0,
        style: str = "plain",
        source_tag: str | None = None,
        source_target: str | None = None,
        offset: int = 0,
        end_offset: int = 0,
    ) -> YAMLRocksAnnotatedStr:
        obj = super().__new__(cls, value)
        obj.__line__ = line
        obj.__column__ = column
        obj.__file__ = config_file
        obj.__end_line__ = end_line
        obj.__end_column__ = end_column
        obj.__style__ = style
        obj.__source_tag__ = source_tag
        obj.__source_target__ = source_target
        obj.__offset__ = offset
        obj.__end_offset__ = end_offset
        return obj


class YAMLRocksAnnotatedInt(int, _SourceTagProvenance):
    """An ``int`` subclass carrying source-location metadata.

    Returned for integer scalars in annotated mode only when
    ``OPT_ANNOTATE_NUMBERS`` is set; without it integers stay plain ``int``. It
    behaves exactly like ``int`` (``isinstance``, equality, and arithmetic all
    work), but ``type(x) is int`` is ``False``. Carries the same attributes as
    :class:`YAMLRocksAnnotatedStr`.
    """

    __line__: int
    __column__: int
    __file__: str | None
    __end_line__: int
    __end_column__: int
    __offset__: int
    __end_offset__: int
    __style__: str

    def __new__(
        cls,
        value: int,
        line: int = 0,
        column: int = 0,
        config_file: str | None = None,
        end_line: int = 0,
        end_column: int = 0,
        style: str = "plain",
        source_tag: str | None = None,
        source_target: str | None = None,
        offset: int = 0,
        end_offset: int = 0,
    ) -> YAMLRocksAnnotatedInt:
        obj = super().__new__(cls, value)
        obj.__line__ = line
        obj.__column__ = column
        obj.__file__ = config_file
        obj.__end_line__ = end_line
        obj.__end_column__ = end_column
        obj.__style__ = style
        obj.__source_tag__ = source_tag
        obj.__source_target__ = source_target
        obj.__offset__ = offset
        obj.__end_offset__ = end_offset
        return obj


class YAMLRocksAnnotatedFloat(float, _SourceTagProvenance):
    """A ``float`` subclass carrying source-location metadata.

    The floating-point counterpart of :class:`YAMLRocksAnnotatedInt`, returned for
    float scalars in annotated mode only when ``OPT_ANNOTATE_NUMBERS`` is set.
    """

    __line__: int
    __column__: int
    __file__: str | None
    __end_line__: int
    __end_column__: int
    __offset__: int
    __end_offset__: int
    __style__: str

    def __new__(
        cls,
        value: float,
        line: int = 0,
        column: int = 0,
        config_file: str | None = None,
        end_line: int = 0,
        end_column: int = 0,
        style: str = "plain",
        source_tag: str | None = None,
        source_target: str | None = None,
        offset: int = 0,
        end_offset: int = 0,
    ) -> YAMLRocksAnnotatedFloat:
        obj = super().__new__(cls, value)
        obj.__line__ = line
        obj.__column__ = column
        obj.__file__ = config_file
        obj.__end_line__ = end_line
        obj.__end_column__ = end_column
        obj.__style__ = style
        obj.__source_tag__ = source_tag
        obj.__source_target__ = source_target
        obj.__offset__ = offset
        obj.__end_offset__ = end_offset
        return obj


__all__ = [
    "OPT_ANNOTATED",
    "OPT_ANNOTATE_NUMBERS",
    "OPT_DUPLICATE_KEYS_ERROR",
    "OPT_DUPLICATE_KEYS_WARN",
    "OPT_ENV_VAR",
    "OPT_ENV_VAR_NOT_FOUND_WARN",
    "OPT_EXPLICIT_END",
    "OPT_EXPLICIT_START",
    "OPT_FLOW_STYLE",
    "OPT_INCLUDES",
    "OPT_INCLUDE_DIR_RECURSIVE",
    "OPT_INDENTLESS_SEQUENCES",
    "OPT_INDENT_2",
    "OPT_INDENT_4",
    "OPT_NAIVE_UTC",
    "OPT_NULL_AS_KEYWORD",
    "OPT_NULL_AS_TILDE",
    "OPT_OMIT_MICROSECONDS",
    "OPT_PASSTHROUGH_DATACLASS",
    "OPT_PASSTHROUGH_DATETIME",
    "OPT_PASSTHROUGH_TAG",
    "OPT_PYYAML_COMPAT",
    "OPT_REJECT_COMPLEX_KEYS",
    "OPT_ROUND_TRIP",
    "OPT_SECRETS",
    "OPT_SECRET_NOT_FOUND_WARN",
    "OPT_SERIALIZE_NUMPY",
    "OPT_SINGLE_QUOTES",
    "OPT_SORT_KEYS",
    "OPT_TIMESTAMPS",
    "OPT_UPGRADE_1_1",
    "OPT_UTC_Z",
    "OPT_YAML_1_1",
    "OPT_YAML_1_1_WARN",
    "YAMLRocksAnnotatedDict",
    "YAMLRocksAnnotatedFloat",
    "YAMLRocksAnnotatedInt",
    "YAMLRocksAnnotatedList",
    "YAMLRocksAnnotatedStr",
    "YAMLRocksCircularIncludeError",
    "YAMLRocksComplexKeyError",
    "YAMLRocksDecodeError",
    "YAMLRocksDocument",
    "YAMLRocksDocumentView",
    "YAMLRocksDuplicateKeyError",
    "YAMLRocksEncodeError",
    "YAMLRocksEnvVarError",
    "YAMLRocksError",
    "YAMLRocksIncludeConfinementError",
    "YAMLRocksIncludeDepthError",
    "YAMLRocksIncludeError",
    "YAMLRocksIncludeNotFoundError",
    "YAMLRocksMapping",
    "YAMLRocksNode",
    "YAMLRocksParseError",
    "YAMLRocksScalar",
    "YAMLRocksSchemaError",
    "YAMLRocksSecretError",
    "YAMLRocksSecretNotFoundError",
    "YAMLRocksSequence",
    "YAMLRocksTag",
    "YAMLRocksTags",
    "YAMLRocksUnserializableError",
    "async_dump",
    "async_load",
    "async_load_all",
    "async_loads",
    "async_loads_all",
    "dump",
    "dump_includes",
    "dump_includes_map",
    "dumps",
    "load",
    "load_all",
    "loads",
    "loads_all",
    "schema_ref",
    "to_json",
    "upgrade",
    "yaml_version",
]


def _read_source(source: Any) -> tuple[bytes | str, str | None]:
    """Return (data, directory) for a path or file-like object.

    ``directory`` is the containing directory when ``source`` is a filesystem
    path (used to default ``include_dir``), otherwise ``None``.
    """
    if hasattr(source, "read"):
        return source.read(), None
    path = os.fspath(source)
    with open(path, "rb") as handle:
        return handle.read(), os.path.dirname(os.path.abspath(path))


@contextlib.contextmanager
def _origin(path: str | None) -> Iterator[None]:
    """Tag any raised ``YAMLRocksError`` with the source file when it has none.

    The Rust core only sees bytes, so it cannot know the path a file was read
    from; the file-oriented entry points fill in ``.file`` here on the way out.
    """
    try:
        yield
    except YAMLRocksError as exc:
        if path is not None and getattr(exc, "file", None) is None:
            exc.file = path
        raise


def load(
    source: Any,
    /,
    *,
    option: int | None = None,
    include_dir: str | os.PathLike[str] | None = None,
    schema: Any | None = None,
    schema_resolver: Callable[[str], Any | None] | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
    on_missing_secret: Callable[[str, str | None, int], None] | None = None,
    on_missing_env_var: Callable[[str, str | None, int], None] | None = None,
) -> Any:
    """Parse YAML from a filesystem path or file-like object.

    The file-oriented counterpart to :func:`loads`. When ``OPT_INCLUDES`` is set
    and ``include_dir`` is not given, includes resolve relative to the file's
    own directory: the natural behavior for a split configuration.
    """
    data, directory = _read_source(source)
    if include_dir is None and (option or 0) & OPT_INCLUDES and directory is not None:
        include_dir = directory
    origin = None if hasattr(source, "read") else os.fspath(source)
    with _origin(origin):
        result = loads(
            data,
            option=option,
            include_dir=include_dir,
            schema=schema,
            schema_resolver=schema_resolver,
            tag_handler=tag_handler,
            tags=tags,
            # Tell the core the real source path so annotated/round-trip nodes
            # from the top-level file report it (not a synthetic placeholder).
            root_path=origin,
            on_missing_secret=on_missing_secret,
            on_missing_env_var=on_missing_env_var,
        )
    # Remember where a round-trip YAMLRocksDocument came from, so ``doc.save()`` (and
    # ``dump(doc)`` with no target) can write back to the same file(s).
    if isinstance(result, YAMLRocksDocument) and not hasattr(source, "read"):
        result.set_origin(os.fspath(source))
    return result


def load_all(
    source: Any,
    /,
    *,
    option: int | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
) -> list[Any]:
    """Parse every document from a path or file-like object into a list."""
    data, _ = _read_source(source)
    origin = None if hasattr(source, "read") else os.fspath(source)
    with _origin(origin):
        return loads_all(data, option=option, tag_handler=tag_handler, tags=tags)


def upgrade(
    data: bytes | bytearray | memoryview | str,
    /,
    *,
    preserve_comments: bool = True,
) -> bytes:
    """Upgrade a YAML 1.1 document to canonical YAML 1.2 and return the bytes.

    Scalars that mean different things between the schemas (``yes``/``no``
    booleans, ``0777`` octals, sexagesimal numbers) are rewritten to their 1.2
    spelling (``true``, ``511``, and so on).

    With ``preserve_comments=True`` (the default) the document is upgraded in
    place, keeping comments, anchors, and layout and changing only the scalars
    that needed it. With ``preserve_comments=False`` the document is reformatted
    from scratch.

    Useful for migrations: e.g. updating Home Assistant configuration files off
    the deprecated YAML 1.1 boolean spellings while leaving everything else
    untouched.
    """
    if preserve_comments:
        # The round-trip document is loaded with OPT_UPGRADE_1_1, so to_yaml()
        # stamps the `%YAML 1.2` directive itself.
        doc = loads(data, option=OPT_ROUND_TRIP | OPT_UPGRADE_1_1)
        return doc.to_yaml()
    # The reformat path goes through the fast dumps emitter, which has no upgrade
    # context, so stamp the version directive here. The directive must be
    # followed by an explicit `---` document start.
    return b"%YAML 1.2\n---\n" + dumps(loads(data, option=OPT_YAML_1_1))


def dump(
    obj: Any,
    target: Any = None,
    /,
    *,
    default: Callable[[Any], Any] | None = None,
    option: int | None = None,
    serializers: dict[type, Callable[[Any], Any]] | None = None,
    width: int | None = None,
) -> None:
    """Serialize ``obj`` as YAML to a filesystem path or file-like object.

    The file-oriented counterpart to :func:`dumps`. Binary streams receive
    ``bytes``; text streams receive ``str``.

    As a convenience, ``dump(doc)`` with no target on a round-trip ``YAMLRocksDocument``
    loaded from disk writes only the changed files back to their origin (see
    :meth:`YAMLRocksDocument.save`).
    """
    if target is None:
        if isinstance(obj, YAMLRocksDocument):
            obj.save()
            return
        raise TypeError(
            "dump() requires a target unless given a YAMLRocksDocument from load()"
        )

    data = dumps(
        obj,
        default=default,
        option=option,
        serializers=serializers,
        width=width,
    )
    if hasattr(target, "write"):
        try:
            target.write(data)
        except TypeError:
            # Text-mode stream: fall back to a decoded string.
            target.write(data.decode("utf-8"))
        return
    with open(os.fspath(target), "wb") as handle:
        handle.write(data)


# -- Async wrappers ----------------------------------------------------------
#
# These cover the operations where running off the event loop thread actually
# pays off: loading (the native scan/parse releases the GIL on byte input, so
# it runs truly in parallel) and the file APIs (which also offload disk I/O).
#
# There are deliberately no `async_dumps` / `async_to_json`: serializing a
# Python object holds the GIL for the object traversal and only frees it for the
# final byte emit, so wrapping it in a thread buys little. Use
# ``asyncio.to_thread(dumps, obj)`` directly in the rare case it matters.
#
# Only the plain fast path releases the GIL fully; with a tag_handler, schema,
# annotated mode, or round-trip, work interleaves Python calls and the loop is
# freed only partially.


async def async_loads(
    data: bytes | bytearray | memoryview | str,
    /,
    *,
    option: int | None = None,
    include_dir: str | os.PathLike[str] | None = None,
    schema: Any | None = None,
    schema_resolver: Callable[[str], Any | None] | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
    on_missing_secret: Callable[[str, str | None, int], None] | None = None,
    on_missing_env_var: Callable[[str, str | None, int], None] | None = None,
) -> Any:
    """Parse YAML off the event loop thread; see :func:`loads`."""
    # Imported lazily, not at module top, so `import yamlrocks` does not pull in
    # asyncio (and its transitive ssl / logging / concurrent.futures), which
    # otherwise dominates import time on the sync path. See issue #52.
    import asyncio

    return await asyncio.to_thread(
        loads,
        data,
        option=option,
        include_dir=include_dir,
        schema=schema,
        schema_resolver=schema_resolver,
        tag_handler=tag_handler,
        tags=tags,
        on_missing_secret=on_missing_secret,
        on_missing_env_var=on_missing_env_var,
    )


async def async_load(
    source: Any,
    /,
    *,
    option: int | None = None,
    include_dir: str | os.PathLike[str] | None = None,
    schema: Any | None = None,
    schema_resolver: Callable[[str], Any | None] | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
    on_missing_secret: Callable[[str, str | None, int], None] | None = None,
    on_missing_env_var: Callable[[str, str | None, int], None] | None = None,
) -> Any:
    """Read and parse a file off the event loop thread; see :func:`load`.

    Both the disk read and the parse run in a worker thread, so this is the
    natural call for loading configuration inside an async application.
    """
    import asyncio  # lazy; see async_loads (issue #52)

    return await asyncio.to_thread(
        load,
        source,
        option=option,
        include_dir=include_dir,
        schema=schema,
        schema_resolver=schema_resolver,
        tag_handler=tag_handler,
        tags=tags,
        on_missing_secret=on_missing_secret,
        on_missing_env_var=on_missing_env_var,
    )


async def async_load_all(
    source: Any,
    /,
    *,
    option: int | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
) -> list[Any]:
    """Read and parse a multi-document file off the event loop; see :func:`load_all`."""
    import asyncio  # lazy; see async_loads (issue #52)

    return await asyncio.to_thread(
        load_all,
        source,
        option=option,
        tag_handler=tag_handler,
        tags=tags,
    )


async def async_loads_all(
    data: bytes | bytearray | memoryview | str,
    /,
    *,
    option: int | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
) -> list[Any]:
    """Parse a multi-document stream off the event loop thread; see :func:`loads_all`."""
    import asyncio  # lazy; see async_loads (issue #52)

    return await asyncio.to_thread(
        loads_all,
        data,
        option=option,
        tag_handler=tag_handler,
        tags=tags,
    )


async def async_dump(
    obj: Any,
    target: Any = None,
    /,
    *,
    default: Callable[[Any], Any] | None = None,
    option: int | None = None,
    serializers: dict[type, Callable[[Any], Any]] | None = None,
    width: int | None = None,
) -> None:
    """Serialize and write YAML off the event loop thread; see :func:`dump`.

    The serialization and the disk write both run in a worker thread.
    """
    import asyncio  # lazy; see async_loads (issue #52)

    await asyncio.to_thread(
        dump,
        obj,
        target,
        default=default,
        option=option,
        serializers=serializers,
        width=width,
    )
