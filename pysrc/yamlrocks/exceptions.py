"""The YAMLRocks exception hierarchy.

Every error YAMLRocks raises derives from :class:`YAMLRocksError`, which carries a
human-readable ``message`` plus the source location (``file``, ``line``,
``column``) whenever it is known. The two category errors also subclass a builtin,
so existing ``except ValueError`` (reading) and ``except TypeError`` (writing)
handlers keep working:

* :class:`YAMLRocksDecodeError` (also a :class:`ValueError`) for anything that goes
  wrong while reading YAML, with a fine-grained subtree for parsing, schema
  validation, includes, secrets, and environment variables;
* :class:`YAMLRocksEncodeError` (also a :class:`TypeError`) for anything that goes
  wrong while writing it.

The classes are defined here, in Python, rather than in the Rust extension: the
hierarchy needs multiple inheritance (a common base *and* the builtin) and a few
structured attributes, both of which are far cleaner to express here. The Rust
core imports these classes and raises them with the attributes populated.
"""

from __future__ import annotations

__all__ = [
    "YAMLRocksCircularIncludeError",
    "YAMLRocksComplexKeyError",
    "YAMLRocksDecodeError",
    "YAMLRocksDuplicateKeyError",
    "YAMLRocksEncodeError",
    "YAMLRocksEnvVarError",
    "YAMLRocksError",
    "YAMLRocksIncludeConfinementError",
    "YAMLRocksIncludeDepthError",
    "YAMLRocksIncludeError",
    "YAMLRocksIncludeNotFoundError",
    "YAMLRocksParseError",
    "YAMLRocksSchemaError",
    "YAMLRocksSecretError",
    "YAMLRocksSecretNotFoundError",
    "YAMLRocksUnserializableError",
]


class YAMLRocksError(Exception):
    """Base class for every error raised by YAMLRocks.

    Carries the human-readable ``message`` and, when known, the source location
    the error points at: ``file`` (path or ``None`` for in-memory input),
    ``line`` and ``column`` (1-based, or ``None`` when not applicable).
    """

    message: str
    file: str | None
    line: int | None
    column: int | None

    def __init__(
        self,
        message: str,
        *,
        file: str | None = None,
        line: int | None = None,
        column: int | None = None,
    ) -> None:
        super().__init__(message)
        self.message = message
        self.file = file
        self.line = line
        self.column = column


# -- Reading YAML ------------------------------------------------------------


class YAMLRocksDecodeError(YAMLRocksError, ValueError):
    """Raised when YAML cannot be read: parsed, resolved, or validated.

    Also a :class:`ValueError`, so ``except ValueError`` keeps catching it.
    """


class YAMLRocksParseError(YAMLRocksDecodeError):
    """The input is not well-formed YAML (a scanner or parser error)."""


class YAMLRocksDuplicateKeyError(YAMLRocksDecodeError):
    """A mapping contains a duplicate key while ``OPT_DUPLICATE_KEYS_ERROR`` is set."""


class YAMLRocksComplexKeyError(YAMLRocksDecodeError):
    """A collection (mapping or sequence) is used as a mapping key while
    ``OPT_REJECT_COMPLEX_KEYS`` is set.

    Such a key is valid YAML and is converted to a hashable Python value
    (``tuple``/``frozenset``) by default; this is raised only when a consumer
    opts into rejecting it instead. ``file``/``line``/``column`` point at the
    offending key. The most common trigger is an unquoted whole-value template
    such as ``state: {{ states('sensor.x') }}``, which YAML reads as a mapping
    used as a key.
    """


class YAMLRocksSchemaError(YAMLRocksDecodeError):
    """A document failed JSON Schema validation.

    ``schema_path`` is the JSON-path of the failing node within the document.
    """

    schema_path: str | None

    def __init__(
        self,
        message: str,
        *,
        file: str | None = None,
        line: int | None = None,
        column: int | None = None,
        schema_path: str | None = None,
    ) -> None:
        super().__init__(message, file=file, line=line, column=column)
        self.schema_path = schema_path


class YAMLRocksIncludeError(YAMLRocksDecodeError):
    """Base for failures resolving the ``!include`` family.

    ``include_stack`` is the chain of ``(file, line)`` pairs that led to the
    failing include, outermost first.
    """

    include_stack: list[tuple[str, int]]

    def __init__(
        self,
        message: str,
        *,
        file: str | None = None,
        line: int | None = None,
        column: int | None = None,
        include_stack: list[tuple[str, int]] | None = None,
    ) -> None:
        super().__init__(message, file=file, line=line, column=column)
        self.include_stack = include_stack if include_stack is not None else []


class YAMLRocksIncludeNotFoundError(YAMLRocksIncludeError):
    """An included file or directory does not exist or cannot be read."""


class YAMLRocksCircularIncludeError(YAMLRocksIncludeError):
    """An include refers back to a file already in the include chain."""


class YAMLRocksIncludeDepthError(YAMLRocksIncludeError):
    """The include chain grew past the maximum allowed depth."""


class YAMLRocksIncludeConfinementError(YAMLRocksIncludeError):
    """An include resolves to a path outside the configured include directory."""


class YAMLRocksSecretError(YAMLRocksDecodeError):
    """Base for ``!secret`` resolution failures."""


class YAMLRocksSecretNotFoundError(YAMLRocksSecretError):
    """A requested secret is not defined in any ``secrets.yaml``."""


class YAMLRocksEnvVarError(YAMLRocksDecodeError):
    """An ``!env_var`` references an undefined variable and gives no default."""


# -- Writing YAML ------------------------------------------------------------


class YAMLRocksEncodeError(YAMLRocksError, TypeError):
    """Raised when a Python value cannot be written as YAML.

    Also a :class:`TypeError`, so ``except TypeError`` keeps catching it.
    """


class YAMLRocksUnserializableError(YAMLRocksEncodeError):
    """A value has no YAML representation and no ``default`` could convert it."""
