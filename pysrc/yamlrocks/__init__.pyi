"""Type stubs for YAMLRocks."""

import os
from collections.abc import Callable, Iterable, Sequence
from typing import Any, Literal, Protocol

# The exception hierarchy is defined (and typed) in yamlrocks.exceptions; re-export
# it here so `yamlrocks.YAMLRocks*Error` resolves through the package.
from yamlrocks.exceptions import (
    YAMLRocksCircularIncludeError as YAMLRocksCircularIncludeError,
)
from yamlrocks.exceptions import (
    YAMLRocksComplexKeyError as YAMLRocksComplexKeyError,
)
from yamlrocks.exceptions import (
    YAMLRocksDecodeError as YAMLRocksDecodeError,
)
from yamlrocks.exceptions import (
    YAMLRocksDuplicateKeyError as YAMLRocksDuplicateKeyError,
)
from yamlrocks.exceptions import (
    YAMLRocksEncodeError as YAMLRocksEncodeError,
)
from yamlrocks.exceptions import (
    YAMLRocksEnvVarError as YAMLRocksEnvVarError,
)
from yamlrocks.exceptions import (
    YAMLRocksError as YAMLRocksError,
)
from yamlrocks.exceptions import (
    YAMLRocksIncludeConfinementError as YAMLRocksIncludeConfinementError,
)
from yamlrocks.exceptions import (
    YAMLRocksIncludeDepthError as YAMLRocksIncludeDepthError,
)
from yamlrocks.exceptions import (
    YAMLRocksIncludeError as YAMLRocksIncludeError,
)
from yamlrocks.exceptions import (
    YAMLRocksIncludeNotFoundError as YAMLRocksIncludeNotFoundError,
)
from yamlrocks.exceptions import (
    YAMLRocksParseError as YAMLRocksParseError,
)
from yamlrocks.exceptions import (
    YAMLRocksSchemaError as YAMLRocksSchemaError,
)
from yamlrocks.exceptions import (
    YAMLRocksSecretError as YAMLRocksSecretError,
)
from yamlrocks.exceptions import (
    YAMLRocksSecretNotFoundError as YAMLRocksSecretNotFoundError,
)
from yamlrocks.exceptions import (
    YAMLRocksUnserializableError as YAMLRocksUnserializableError,
)

class _Readable(Protocol):
    """A file-like object opened for reading text or bytes."""

    def read(self, /) -> str | bytes: ...

class _Writable(Protocol):
    """A file-like object opened for writing text or bytes."""

    def write(self, data: Any, /) -> Any: ...

_Source = str | os.PathLike[str] | _Readable

# Schema and YAML version. On reading, selects how scalars resolve; on dumping,
# OPT_YAML_1_1 quotes the scalars only strict 1.1 reads as non-strings (bare
# `y`/`n`, sexagesimal) so the output re-reads identically under YAML 1.1.
OPT_YAML_1_1: int
OPT_PYYAML_COMPAT: int
OPT_UPGRADE_1_1: int
OPT_YAML_1_1_WARN: int
# Reading: result shape.
OPT_ROUND_TRIP: int
OPT_ANNOTATED: int
OPT_ANNOTATE_NUMBERS: int
# Reading: includes.
OPT_INCLUDES: int
OPT_INCLUDE_DIR_RECURSIVE: int
# Reading: config tags (secrets and environment variables).
OPT_SECRETS: int
OPT_ENV_VAR: int
OPT_SECRET_NOT_FOUND_WARN: int
OPT_ENV_VAR_NOT_FOUND_WARN: int
# Reading: tags and keys.
OPT_PASSTHROUGH_TAG: int
OPT_DUPLICATE_KEYS_ERROR: int
OPT_DUPLICATE_KEYS_WARN: int
OPT_REJECT_COMPLEX_KEYS: int
# Writing: layout.
OPT_INDENT_2: int
OPT_INDENT_4: int
OPT_INDENTLESS_SEQUENCES: int
OPT_FLOW_STYLE: int
OPT_SORT_KEYS: int
OPT_EXPLICIT_START: int
OPT_EXPLICIT_END: int
# Writing: scalar style.
OPT_SINGLE_QUOTES: int
OPT_NULL_AS_KEYWORD: int
OPT_NULL_AS_TILDE: int
# Writing: type serialization.
OPT_SERIALIZE_NUMPY: int
OPT_PASSTHROUGH_DATETIME: int
OPT_PASSTHROUGH_DATACLASS: int
OPT_OMIT_MICROSECONDS: int
OPT_NAIVE_UTC: int
OPT_UTC_Z: int
OPT_TIMESTAMPS: int

class YAMLRocksDocumentView:
    """A live proxy onto a nested mapping/sequence of a ``YAMLRocksDocument``.

    Reads and writes navigate the underlying AST, so edits are retained and
    reflected by ``YAMLRocksDocument.to_yaml`` and the ``dump_includes`` family.
    """

    def __len__(self) -> int: ...
    def __getitem__(self, key: str | int) -> Any: ...
    def __setitem__(self, key: str | int, value: Any) -> None: ...
    def __delitem__(self, key: str | int) -> None: ...
    def __contains__(self, key: str) -> bool: ...
    def get(self, key: str | int, default: Any = None) -> Any: ...
    def keys(self) -> list[Any]: ...
    def unwrap(self) -> Any: ...
    def to_dict(self) -> Any: ...
    def walk(self) -> list[tuple[tuple[str | int, ...], Any]]: ...
    def range(self) -> tuple[int, int, int, int]: ...
    def to_yaml(self) -> bytes: ...
    @property
    def node(self) -> YAMLRocksNode: ...

class YAMLRocksNode:
    """A metadata-bearing handle onto a single node of a ``YAMLRocksDocument``.

    Unlike item access, indexing a ``YAMLRocksNode`` always returns another ``YAMLRocksNode``
    (scalars included), so comments, source location, style, anchor, and tag are
    reachable for any node. Obtain the root cursor with ``YAMLRocksDocument.node``.
    """

    value: Any
    comment: str | None
    comment_before: str | None
    comment_after: str | None
    anchor: str | None
    @property
    def line(self) -> int: ...
    @property
    def column(self) -> int: ...
    def range(self) -> tuple[int, int, int, int]: ...
    @property
    def offset(self) -> int: ...
    @property
    def end_offset(self) -> int: ...
    @property
    def file(self) -> str | None: ...
    @property
    def style(self) -> str: ...
    @property
    def tag(self) -> str | None: ...
    @property
    def source_tag(self) -> str | None: ...
    @property
    def source_target(self) -> str | None: ...
    @property
    def is_secret(self) -> bool: ...
    @property
    def is_env_var(self) -> bool: ...
    @property
    def is_include(self) -> bool: ...
    @property
    def is_alias(self) -> bool: ...
    @property
    def target(self) -> YAMLRocksNode | None: ...
    @property
    def aliases(self) -> list[YAMLRocksNode]: ...
    def detach(self) -> YAMLRocksNode: ...
    def make_alias(self, name: str) -> None: ...
    def __getitem__(self, key: str | int) -> YAMLRocksNode: ...
    def __contains__(self, key: str) -> bool: ...

class YAMLRocksDocument:
    origin: str | None
    def __len__(self) -> int: ...
    def __getitem__(self, key: str | int) -> Any: ...
    def __setitem__(self, key: str | int, value: Any) -> None: ...
    def __delitem__(self, key: str | int) -> None: ...
    def __contains__(self, key: str) -> bool: ...
    def get(self, key: str | int, default: Any = None) -> Any: ...
    def keys(self) -> list[Any]: ...
    def set_origin(self, path: str) -> None: ...
    def save(self, path: str | None = None) -> list[str]: ...
    def range(self) -> tuple[int, int, int, int]: ...
    def to_yaml(self) -> bytes: ...
    def to_dict(self) -> Any: ...
    def walk(self) -> list[tuple[tuple[str | int, ...], Any]]: ...
    def locate(self, path: Sequence[str | int]) -> YAMLRocksNode | None: ...
    @property
    def node(self) -> YAMLRocksNode: ...
    @property
    def anchors(self) -> dict[str, YAMLRocksNode]: ...

class _SourceTagProvenance:
    __source_tag__: str | None
    __source_target__: str | None
    @property
    def is_secret(self) -> bool: ...
    @property
    def is_env_var(self) -> bool: ...
    @property
    def is_include(self) -> bool: ...

class YAMLRocksAnnotatedDict(dict[Any, Any]):
    __line__: int
    __column__: int
    __file__: str | None
    __end_line__: int
    __end_column__: int
    __offset__: int
    __end_offset__: int
    __source_tag__: str | None
    __source_target__: str | None
    @property
    def is_secret(self) -> bool: ...
    @property
    def is_env_var(self) -> bool: ...
    @property
    def is_include(self) -> bool: ...

class YAMLRocksAnnotatedList(list[Any]):
    __line__: int
    __column__: int
    __file__: str | None
    __end_line__: int
    __end_column__: int
    __offset__: int
    __end_offset__: int
    __source_tag__: str | None
    __source_target__: str | None
    @property
    def is_secret(self) -> bool: ...
    @property
    def is_env_var(self) -> bool: ...
    @property
    def is_include(self) -> bool: ...

class YAMLRocksAnnotatedStr(str, _SourceTagProvenance):
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
        line: int = ...,
        column: int = ...,
        config_file: str | None = ...,
        end_line: int = ...,
        end_column: int = ...,
        style: str = ...,
        source_tag: str | None = ...,
        source_target: str | None = ...,
        offset: int = ...,
        end_offset: int = ...,
    ) -> YAMLRocksAnnotatedStr: ...

class YAMLRocksAnnotatedInt(int, _SourceTagProvenance):
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
        line: int = ...,
        column: int = ...,
        config_file: str | None = ...,
        end_line: int = ...,
        end_column: int = ...,
        style: str = ...,
        source_tag: str | None = ...,
        source_target: str | None = ...,
        offset: int = ...,
        end_offset: int = ...,
    ) -> YAMLRocksAnnotatedInt: ...

class YAMLRocksAnnotatedFloat(float, _SourceTagProvenance):
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
        line: int = ...,
        column: int = ...,
        config_file: str | None = ...,
        end_line: int = ...,
        end_column: int = ...,
        style: str = ...,
        source_tag: str | None = ...,
        source_target: str | None = ...,
        offset: int = ...,
        end_offset: int = ...,
    ) -> YAMLRocksAnnotatedFloat: ...

class YAMLRocksTag:
    tag: str
    value: Any
    def __init__(self, tag: str, value: Any) -> None: ...

# Node descriptors returned by a ``dumps(represent=...)`` callback.
_Style = Literal["auto", "plain", "single", "double", "literal", "folded"]

class YAMLRocksScalar:
    def __init__(
        self, value: str, *, tag: str | None = None, style: _Style = "auto"
    ) -> None: ...
    @property
    def value(self) -> str: ...
    @property
    def tag(self) -> str | None: ...
    @property
    def style(self) -> _Style: ...

class YAMLRocksSequence:
    def __init__(
        self, items: Iterable[Any], *, tag: str | None = None, flow: bool | None = None
    ) -> None: ...
    @property
    def items(self) -> list[Any] | tuple[Any, ...]: ...
    @property
    def tag(self) -> str | None: ...
    @property
    def flow(self) -> bool | None: ...

class YAMLRocksMapping:
    def __init__(
        self,
        pairs: Iterable[tuple[Any, Any]],
        *,
        tag: str | None = None,
        flow: bool | None = None,
    ) -> None: ...
    @property
    def pairs(self) -> list[tuple[Any, Any]] | tuple[tuple[Any, Any], ...]: ...
    @property
    def tag(self) -> str | None: ...
    @property
    def flow(self) -> bool | None: ...

_Node = YAMLRocksScalar | YAMLRocksSequence | YAMLRocksMapping

class YAMLRocksTags(dict[str, Callable[[Any], Any]]):
    def register(
        self,
        tag: str,
        func: Callable[[Any], Any] | None = ...,
    ) -> Callable[[Any], Any]: ...

def loads(
    data: bytes | bytearray | memoryview | str,
    /,
    *,
    option: int | None = None,
    include_dir: str | os.PathLike[str] | None = None,
    schema: Any | None = None,
    schema_resolver: Callable[[str], Any | None] | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
    root_path: str | os.PathLike[str] | None = None,
    on_missing_secret: Callable[[str, str | None, int], None] | None = None,
    on_missing_env_var: Callable[[str, str | None, int], None] | None = None,
) -> (
    dict[str, Any] | list[Any] | str | int | float | bool | None | YAMLRocksDocument
): ...
def load(
    source: _Source,
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
) -> (
    dict[str, Any] | list[Any] | str | int | float | bool | None | YAMLRocksDocument
): ...
def schema_ref(
    data: bytes | bytearray | memoryview | str,
    /,
) -> str | None: ...
def yaml_version(
    data: bytes | bytearray | memoryview | str,
    /,
) -> str | None: ...
def load_all(
    source: _Source,
    /,
    *,
    option: int | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
) -> list[Any]: ...
def upgrade(
    data: bytes | bytearray | memoryview | str,
    /,
    *,
    preserve_comments: bool = True,
) -> bytes: ...
def dump(
    obj: Any,
    target: str | os.PathLike[str] | _Writable | None = None,
    /,
    *,
    default: Callable[[Any], Any] | None = None,
    option: int | None = None,
    serializers: dict[type, Callable[[Any], Any]] | None = None,
    width: int | None = None,
    represent: Callable[[Any], _Node | None] | None = None,
) -> None: ...
def loads_all(
    data: bytes | bytearray | memoryview | str,
    /,
    *,
    option: int | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
) -> list[Any]: ...
def dumps(
    obj: Any,
    /,
    *,
    default: Callable[[Any], Any] | None = None,
    option: int | None = None,
    serializers: dict[type, Callable[[Any], Any]] | None = None,
    width: int | None = None,
    represent: Callable[[Any], _Node | None] | None = None,
) -> bytes: ...
def to_json(
    obj: Any,
    /,
    *,
    default: Callable[[Any], Any] | None = None,
    option: int | None = None,
) -> bytes: ...
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
) -> (
    dict[str, Any] | list[Any] | str | int | float | bool | None | YAMLRocksDocument
): ...
async def async_load(
    source: _Source,
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
) -> (
    dict[str, Any] | list[Any] | str | int | float | bool | None | YAMLRocksDocument
): ...
async def async_load_all(
    source: _Source,
    /,
    *,
    option: int | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
) -> list[Any]: ...
async def async_loads_all(
    data: bytes | bytearray | memoryview | str,
    /,
    *,
    option: int | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
) -> list[Any]: ...
async def async_dump(
    obj: Any,
    target: str | os.PathLike[str] | _Writable | None = None,
    /,
    *,
    default: Callable[[Any], Any] | None = None,
    option: int | None = None,
    serializers: dict[type, Callable[[Any], Any]] | None = None,
    width: int | None = None,
    represent: Callable[[Any], _Node | None] | None = None,
) -> None: ...
def dump_includes(
    doc: YAMLRocksDocument,
    /,
    *,
    include_dir: str | os.PathLike[str] | None = None,
) -> None: ...
def dump_includes_map(
    doc: YAMLRocksDocument,
    /,
) -> dict[str, bytes]: ...
