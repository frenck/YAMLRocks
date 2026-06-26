"""Native include resolution and writable includes."""

from __future__ import annotations

import os

import pytest

import yamlrocks

RT_INC = yamlrocks.OPT_ROUND_TRIP | yamlrocks.OPT_INCLUDES


@pytest.fixture
def config_dir(tmp_path):
    (tmp_path / "automations.yaml").write_text(
        "- alias: morning  # turn on\n  trigger: sunrise\n"
        "- alias: night\n  trigger: sunset\n"
    )
    (tmp_path / "secrets.yaml").write_text("api_key: SECRET123\n")
    root = (
        "# Main config\n"
        "automation: !include automations.yaml\n"
        "secrets: !include secrets.yaml\n"
        "name: My Home\n"
    )
    return tmp_path, root


def test_fast_path_include_resolves(tmp_path):
    """Fast-path loading inlines an !include file's parsed content."""
    (tmp_path / "sub.yaml").write_text("nested:\n  deep: 1\n")
    result = yamlrocks.loads(
        b"top: !include sub.yaml\nx: 1\n",
        option=yamlrocks.OPT_INCLUDES,
        include_dir=str(tmp_path),
    )
    assert result == {"top": {"nested": {"deep": 1}}, "x": 1}


def test_roundtrip_resolves_for_reading(config_dir):
    """Round-trip include mode resolves included content for reading."""
    tmp_path, root = config_dir
    doc = yamlrocks.loads(root.encode(), option=RT_INC, include_dir=str(tmp_path))
    assert doc["automation"][0]["alias"] == "morning"
    assert doc["name"] == "My Home"


def test_roundtrip_root_view_keeps_directives(config_dir):
    """Emitting the root view keeps !include directives without inlining."""
    tmp_path, root = config_dir
    doc = yamlrocks.loads(root.encode(), option=RT_INC, include_dir=str(tmp_path))
    out = doc.to_yaml().decode()
    assert "!include automations.yaml" in out
    assert "!include secrets.yaml" in out
    # The included content must not be inlined into the root view.
    assert "sunrise" not in out


def test_write_back_targets_included_file(config_dir):
    """Editing included data writes changes back to the included file."""
    tmp_path, root = config_dir
    doc = yamlrocks.loads(root.encode(), option=RT_INC, include_dir=str(tmp_path))
    doc["automation"][1]["trigger"] = "civil_dusk"

    changes = yamlrocks.dump_includes_map(doc)
    names = {os.path.basename(p) for p in changes}
    assert names == {"automations.yaml", "secrets.yaml"}

    automations = next(v for p, v in changes.items() if p.endswith("automations.yaml"))
    assert b"civil_dusk" in automations
    # An unrelated comment in the included file is preserved.
    assert b"# turn on" in automations


def test_same_file_included_twice_with_divergent_edits_is_rejected(tmp_path):
    """A file included twice cannot be written back with two different versions.

    Editing only one of the two occurrences makes the file's two subtrees re-emit
    to different bytes; write-back refuses rather than silently dropping one edit.
    An unmodified (identical) double-include still writes back cleanly.
    """
    (tmp_path / "shared.yaml").write_text("v: 1\n")
    root = b"a: !include shared.yaml\nb: !include shared.yaml\n"

    # Unmodified: both occurrences are identical, so there is no conflict.
    doc = yamlrocks.loads(root, option=RT_INC, include_dir=str(tmp_path))
    assert yamlrocks.dump_includes_map(doc)  # succeeds

    # Divergent: edit only the first occurrence.
    doc2 = yamlrocks.loads(root, option=RT_INC, include_dir=str(tmp_path))
    doc2["a"]["v"] = 999
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="more than once"):
        yamlrocks.dump_includes_map(doc2)


def test_unmodified_include_reemits_verbatim(tmp_path):
    """An unmodified included file writes back byte-for-byte.

    The write-back uses each file's cached source, so a file the edit never
    touched is reproduced exactly - even formatting the AST emitter would
    otherwise normalize, like a compact block sequence and extra spacing before
    an inline comment.
    """
    included = (
        b"- alias: a    # extra spaces before the hash are kept\n"
        b"  trigger:\n"
        b"  - platform: x\n"
    )
    (tmp_path / "automations.yaml").write_bytes(included)
    root = b"automation: !include automations.yaml\n"
    doc = yamlrocks.loads(root, option=RT_INC, include_dir=str(tmp_path))

    changes = yamlrocks.dump_includes_map(doc)
    written = next(v for p, v in changes.items() if p.endswith("automations.yaml"))
    assert written == included  # verbatim, not reflowed


def test_editing_one_include_leaves_siblings_byte_identical(tmp_path):
    """Editing one included file re-emits only that file; siblings stay verbatim.

    The edited file is rendered from the AST (so the change lands) while keeping
    its compact sequence style; the untouched sibling is byte-for-byte identical.
    """
    autos = b"- alias: a\n  trigger:\n  - platform: x\n"
    scripts = b"greet:\n  sequence:\n  - service: notify.x   # keep spacing\n"
    (tmp_path / "automations.yaml").write_bytes(autos)
    (tmp_path / "scripts.yaml").write_bytes(scripts)
    root = b"automation: !include automations.yaml\nscript: !include scripts.yaml\n"
    doc = yamlrocks.loads(root, option=RT_INC, include_dir=str(tmp_path))

    doc["automation"][0]["alias"] = "edited"

    changes = yamlrocks.dump_includes_map(doc)
    written_autos = next(
        v for p, v in changes.items() if p.endswith("automations.yaml")
    )
    written_scripts = next(v for p, v in changes.items() if p.endswith("scripts.yaml"))
    assert b"alias: edited" in written_autos  # the edit landed
    assert b"  - platform: x" in written_autos  # compact sequence style kept
    assert written_scripts == scripts  # untouched sibling unchanged, byte for byte


def test_dump_includes_writes_files(config_dir):
    """dump_includes writes edited included content to disk."""
    tmp_path, root = config_dir
    doc = yamlrocks.loads(root.encode(), option=RT_INC, include_dir=str(tmp_path))
    doc["automation"][1]["trigger"] = "civil_dusk"
    yamlrocks.dump_includes(doc, include_dir=str(tmp_path))

    written = (tmp_path / "automations.yaml").read_text()
    assert "civil_dusk" in written


def test_dump_includes_requires_include_document():
    """dump_includes_map rejects a document loaded without includes."""
    doc = yamlrocks.loads(b"a: 1\n", option=yamlrocks.OPT_ROUND_TRIP)
    with pytest.raises(ValueError):
        yamlrocks.dump_includes_map(doc)


@pytest.mark.skipif(os.name != "posix", reason="symlink semantics differ on Windows")
def test_dump_includes_does_not_write_through_a_swapped_symlink(config_dir):
    """A source file swapped for a symlink between load and write-back must be
    replaced, not followed: the write stays in the tree and the link target
    outside it is left untouched (an atomic, symlink-safe write)."""
    tmp_path, root = config_dir
    doc = yamlrocks.loads(root.encode(), option=RT_INC, include_dir=str(tmp_path))
    doc["automation"][1]["trigger"] = "civil_dusk"

    # Stage the attack: replace the tracked source file with a symlink that points
    # at a file outside the configuration tree.
    outside = tmp_path.parent / "outside.txt"
    outside.write_text("do-not-clobber\n")
    target = tmp_path / "automations.yaml"
    target.unlink()
    target.symlink_to(outside)

    yamlrocks.dump_includes(doc, include_dir=str(tmp_path))

    # The outside file is untouched, and the include path is now a real file.
    assert outside.read_text() == "do-not-clobber\n"
    assert not target.is_symlink()
    assert "civil_dusk" in target.read_text()


def test_include_dir_list(tmp_path):
    """!include_dir_list yields one entry per file in sorted order."""
    # !include_dir_list yields one entry per file (in sorted filename order).
    pkgs = tmp_path / "packages"
    pkgs.mkdir()
    (pkgs / "a.yaml").write_text("name: first\n")
    (pkgs / "b.yaml").write_text("name: second\n")
    result = yamlrocks.loads(
        b"items: !include_dir_list packages\n",
        option=yamlrocks.OPT_INCLUDES,
        include_dir=str(tmp_path),
    )
    assert result["items"] == [{"name": "first"}, {"name": "second"}]


def test_include_dir_merge_list(tmp_path):
    """!include_dir_merge_list flattens per-file lists into one list."""
    # !include_dir_merge_list flattens the per-file lists into one.
    pkgs = tmp_path / "packages"
    pkgs.mkdir()
    (pkgs / "a.yaml").write_text("- 1\n- 2\n")
    (pkgs / "b.yaml").write_text("- 3\n")
    result = yamlrocks.loads(
        b"items: !include_dir_merge_list packages\n",
        option=yamlrocks.OPT_INCLUDES,
        include_dir=str(tmp_path),
    )
    assert sorted(result["items"]) == [1, 2, 3]


def test_include_dir_named(tmp_path):
    """!include_dir_named maps each file's stem to its content."""
    pkgs = tmp_path / "things"
    pkgs.mkdir()
    (pkgs / "first.yaml").write_text("value: 1\n")
    (pkgs / "second.yaml").write_text("value: 2\n")
    result = yamlrocks.loads(
        b"things: !include_dir_named things\n",
        option=yamlrocks.OPT_INCLUDES,
        include_dir=str(tmp_path),
    )
    assert result["things"] == {
        "first": {"value": 1},
        "second": {"value": 2},
    }


def test_include_dir_merge_named(tmp_path):
    """!include_dir_merge_named folds the per-file mappings into one."""
    pkgs = tmp_path / "packages"
    pkgs.mkdir()
    (pkgs / "a.yaml").write_text("sensor_a: 1\n")
    (pkgs / "b.yaml").write_text("sensor_b: 2\n")
    result = yamlrocks.loads(
        b"all: !include_dir_merge_named packages\n",
        option=yamlrocks.OPT_INCLUDES,
        include_dir=str(tmp_path),
    )
    assert result["all"] == {"sensor_a": 1, "sensor_b": 2}


def test_include_dir_merge_named_collision_is_last_file_wins(tmp_path):
    """On a key collision, the file sorted last by name wins.

    Documents the defined merge order: files are read in sorted filename order
    and folded into one mapping, so a later file's key overrides an earlier
    one. ``OPT_DUPLICATE_KEYS_ERROR`` governs duplicates *within* a single
    document and does not turn cross-file overrides into an error.
    """
    pkgs = tmp_path / "packages"
    pkgs.mkdir()
    (pkgs / "a.yaml").write_text("shared: from_a\n")
    (pkgs / "z.yaml").write_text("shared: from_z\n")
    result = yamlrocks.loads(
        b"all: !include_dir_merge_named packages\n",
        option=yamlrocks.OPT_INCLUDES,
        include_dir=str(tmp_path),
    )
    assert result["all"] == {"shared": "from_z"}


def test_include_dir_resolves_nested_directives(tmp_path):
    """A file loaded via !include_dir_named resolves its own !include, like
    a plain !include does."""
    pkgs = tmp_path / "pkgs"
    pkgs.mkdir()
    (pkgs / "a.yaml").write_text("inner: !include ../leaf.yaml\n")
    (tmp_path / "leaf.yaml").write_text("resolved: yes\n")
    result = yamlrocks.loads(
        b"all: !include_dir_named pkgs\n",
        option=yamlrocks.OPT_INCLUDES,
        include_dir=str(tmp_path),
    )
    assert result == {"all": {"a": {"inner": {"resolved": "yes"}}}}


def test_missing_include_raises(tmp_path):
    """A missing included file raises YAMLRocksDecodeError."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError):
        yamlrocks.loads(
            b"x: !include nope.yaml\n",
            option=yamlrocks.OPT_INCLUDES,
            include_dir=str(tmp_path),
        )


def test_missing_relative_include_is_not_found_not_confinement(tmp_path, monkeypatch):
    """A missing in-tree file with a relative include_dir is not-found, not a
    confinement escape. Regression: a relative dir left the candidate path
    relative, so it failed the absolute-base check and was misreported as an
    attempt to escape the include directory."""
    monkeypatch.chdir(tmp_path)
    with pytest.raises(yamlrocks.YAMLRocksIncludeNotFoundError):
        yamlrocks.loads(
            b"x: !include missing.yaml\n",
            option=yamlrocks.OPT_INCLUDES,
            include_dir=".",
        )
    # A genuine escape is still a confinement error, even with a relative dir.
    with pytest.raises(yamlrocks.YAMLRocksIncludeConfinementError):
        yamlrocks.loads(
            b"x: !include ../escape.yaml\n",
            option=yamlrocks.OPT_INCLUDES,
            include_dir=".",
        )


def test_include_resolver_composes_tag_families(tmp_path, monkeypatch):
    """The include resolver expands !include, !secret, and !env_var together.

    Each tag family is independently gated, but a single load can enable all of
    them; this verifies they compose. The tags themselves are covered in depth by
    test_secret_tag and test_env_var_tag.
    """
    monkeypatch.setenv("YAMLROCKS_HOST", "example.com")
    (tmp_path / "secrets.yaml").write_text("api_key: SECRET123\n")
    (tmp_path / "sub.yaml").write_text("host: !env_var YAMLROCKS_HOST\n")
    data = yamlrocks.loads(
        b"s: !include sub.yaml\napi: !secret api_key\n",
        option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_SECRETS | yamlrocks.OPT_ENV_VAR,
        include_dir=str(tmp_path),
    )
    assert data == {"s": {"host": "example.com"}, "api": "SECRET123"}


def test_root_includes_resolve_relative_to_root_file_directory(tmp_path):
    """A `!include` in the top-level file resolves relative to that file's own
    directory, even when `include_dir` points at a different (ancestor) directory.

    Registering the root under its real path (so it reports the right source
    file) means root includes follow the root file's location; confinement to
    `include_dir` still applies, since the target stays within it here.
    """
    sub = tmp_path / "sub"
    sub.mkdir()
    (sub / "data.yaml").write_text("value: 42\n")
    (sub / "main.yaml").write_text("section: !include data.yaml\n")
    data = yamlrocks.load(
        str(sub / "main.yaml"),
        option=yamlrocks.OPT_INCLUDES,
        include_dir=str(tmp_path),  # ancestor dir, differs from the file's own dir
    )
    assert data == {"section": {"value": 42}}


# -- Directory-include enumeration: recursion, secrets, dotfiles ---------------


def test_include_dir_list_is_flat_by_default(tmp_path):
    """Without OPT_INCLUDE_DIR_RECURSIVE only the top level is read."""
    (tmp_path / "t").mkdir()
    (tmp_path / "t" / "zero.yaml").write_text("zero")
    (tmp_path / "t" / "sub").mkdir()
    (tmp_path / "t" / "sub" / "one.yaml").write_text("one")
    (tmp_path / "c.yaml").write_text("k: !include_dir_list t")
    data = yamlrocks.load(str(tmp_path / "c.yaml"), option=yamlrocks.OPT_INCLUDES)
    assert data == {"k": ["zero"]}


def test_include_dir_list_recursive_opt_walks_subdirs(tmp_path):
    """OPT_INCLUDE_DIR_RECURSIVE descends, top level before deeper, each sorted."""
    (tmp_path / "t").mkdir()
    (tmp_path / "t" / "zero.yaml").write_text("zero")
    (tmp_path / "t" / "sub").mkdir()
    (tmp_path / "t" / "sub" / "one.yaml").write_text("one")
    (tmp_path / "c.yaml").write_text("k: !include_dir_list t")
    data = yamlrocks.load(
        str(tmp_path / "c.yaml"),
        option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_INCLUDE_DIR_RECURSIVE,
    )
    assert data == {"k": ["zero", "one"]}


def test_include_dir_named_recursive_keys_by_stem(tmp_path):
    """Recursive !include_dir_named keys every file by its basename stem."""
    (tmp_path / "t").mkdir()
    (tmp_path / "t" / "a.yaml").write_text("x: 1")
    (tmp_path / "t" / "sub").mkdir()
    (tmp_path / "t" / "sub" / "b.yaml").write_text("y: 2")
    (tmp_path / "c.yaml").write_text("k: !include_dir_named t")
    data = yamlrocks.load(
        str(tmp_path / "c.yaml"),
        option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_INCLUDE_DIR_RECURSIVE,
    )
    assert data == {"k": {"a": {"x": 1}, "b": {"y": 2}}}


def test_include_dir_skips_secrets_yaml_only_with_secrets_opt(tmp_path):
    """secrets.yaml is content without OPT_SECRETS, skipped with it."""
    (tmp_path / "t").mkdir()
    (tmp_path / "t" / "first.yaml").write_text("one")
    (tmp_path / "t" / "secrets.yaml").write_text("some_secret: value")
    (tmp_path / "c.yaml").write_text("k: !include_dir_named t")
    without = yamlrocks.load(str(tmp_path / "c.yaml"), option=yamlrocks.OPT_INCLUDES)
    assert "secrets" in without["k"]
    with_secrets = yamlrocks.load(
        str(tmp_path / "c.yaml"),
        option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_SECRETS,
    )
    assert with_secrets == {"k": {"first": "one"}}


def test_include_dir_skips_hidden_files_and_dirs(tmp_path):
    """Dotfiles and dot-directories are skipped during the walk."""
    (tmp_path / "t").mkdir()
    (tmp_path / "t" / "a.yaml").write_text("1")
    (tmp_path / "t" / ".hidden.yaml").write_text("2")
    (tmp_path / "t" / ".ignore").mkdir()
    (tmp_path / "t" / ".ignore" / "c.yaml").write_text("3")
    (tmp_path / "c.yaml").write_text("k: !include_dir_list t")
    data = yamlrocks.load(
        str(tmp_path / "c.yaml"),
        option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_INCLUDE_DIR_RECURSIVE,
    )
    assert data == {"k": [1]}


def test_empty_single_include_resolves_to_empty_mapping(tmp_path):
    """An empty !include file normalizes to {} (not None)."""
    (tmp_path / "empty.yaml").write_text("")
    data = yamlrocks.loads(
        b"k: !include empty.yaml",
        option=yamlrocks.OPT_INCLUDES,
        include_dir=str(tmp_path),
    )
    assert data == {"k": {}}


def test_empty_dir_named_entry_resolves_to_empty_mapping(tmp_path):
    """An empty file under !include_dir_named maps to {} (list forms exclude it)."""
    (tmp_path / "t").mkdir()
    (tmp_path / "t" / "full.yaml").write_text("a: 1\n")
    (tmp_path / "t" / "empty.yaml").write_text("")
    (tmp_path / "c.yaml").write_text("k: !include_dir_named t")
    data = yamlrocks.load(str(tmp_path / "c.yaml"), option=yamlrocks.OPT_INCLUDES)
    assert data == {"k": {"full": {"a": 1}, "empty": {}}}


@pytest.mark.parametrize(
    "tag",
    [
        "!include",
        "!include_dir_named",
        "!include_dir_list",
        "!include_dir_merge_named",
        "!include_dir_merge_list",
    ],
)
def test_include_tag_without_argument_errors(tag):
    """Every include tag requires a target; a bare tag is an error."""
    with pytest.raises(yamlrocks.YAMLRocksIncludeError, match="needs an argument"):
        yamlrocks.loads(f"key: {tag}".encode(), option=yamlrocks.OPT_INCLUDES)


def test_dir_named_key_annotated_with_included_file_location(tmp_path):
    """A synthetic dir-named key points at the included file's own line 1."""
    (tmp_path / "pkgs").mkdir()
    (tmp_path / "pkgs" / "alpha.yaml").write_text("x: 1\n")
    (tmp_path / "c.yaml").write_text("k: !include_dir_named pkgs")
    doc = yamlrocks.load(
        str(tmp_path / "c.yaml"),
        option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_ANNOTATED,
    )
    (key,) = list(doc["k"])
    assert key == "alpha"
    assert key.__file__.endswith(os.path.join("pkgs", "alpha.yaml"))
    assert key.__line__ == 1


# -- Structural validation follows !include boundaries (annotated mode) --------

# A document with an over-indented block (a block collection in mapping-key
# position): rejected as a root, and now equally when reached through an include.
_BAD = b"- platform: x\n      option1: abc\n"


def test_invalid_root_and_included_raise_identical_error(tmp_path):
    """The same malformed file errors identically as a root and via !include."""
    (tmp_path / "bad.yaml").write_bytes(_BAD)
    opt = yamlrocks.OPT_ANNOTATED | yamlrocks.OPT_INCLUDES | yamlrocks.OPT_YAML_1_1

    with pytest.raises(yamlrocks.YAMLRocksParseError) as root:
        yamlrocks.loads(_BAD, option=opt, root_path=str(tmp_path / "bad.yaml"))

    (tmp_path / "configuration.yaml").write_text("iot_domain: !include bad.yaml\n")
    with pytest.raises(yamlrocks.YAMLRocksParseError) as via_include:
        yamlrocks.load(str(tmp_path / "configuration.yaml"), option=opt)

    # Same message and location; the included case points at the included file.
    assert str(root.value) == str(via_include.value)
    assert (root.value.line, root.value.column) == (
        via_include.value.line,
        via_include.value.column,
    )
    assert via_include.value.file.endswith("bad.yaml")
    assert (via_include.value.line, via_include.value.column) == (2, 14)


@pytest.mark.parametrize(
    ("directive", "make"),
    [
        ("k: !include bad.yaml", None),
        ("k: !include_dir_list d", "dir"),
        ("k: !include_dir_merge_list d", "dir"),
        ("k: !include_dir_named d", "dir"),
        ("k: !include_dir_merge_named d", "dir"),
    ],
)
def test_structural_error_in_included_file_is_reported(tmp_path, directive, make):
    """Every include form surfaces a structural error in a pulled-in file as a
    YAMLRocksParseError pointing at that file (not an opaque later failure)."""
    opt = yamlrocks.OPT_ANNOTATED | yamlrocks.OPT_INCLUDES | yamlrocks.OPT_YAML_1_1
    if make == "dir":
        (tmp_path / "d").mkdir()
        (tmp_path / "d" / "bad.yaml").write_bytes(_BAD)
    else:
        (tmp_path / "bad.yaml").write_bytes(_BAD)
    (tmp_path / "c.yaml").write_text(directive + "\n")

    with pytest.raises(yamlrocks.YAMLRocksParseError) as err:
        yamlrocks.load(str(tmp_path / "c.yaml"), option=opt)
    assert err.value.file.endswith("bad.yaml")
    assert "block collection cannot be a mapping key" in str(err.value)


def test_valid_includes_unaffected_by_validation(tmp_path):
    """Validation does not disturb a valid include graph."""
    (tmp_path / "good.yaml").write_text("- platform: x\n")
    (tmp_path / "c.yaml").write_text("iot: !include good.yaml\n")
    opt = yamlrocks.OPT_ANNOTATED | yamlrocks.OPT_INCLUDES
    assert yamlrocks.load(str(tmp_path / "c.yaml"), option=opt) == {
        "iot": [{"platform": "x"}]
    }


# A collection (mapping or sequence) used as a mapping key is unhashable as a
# Python dict key, so YAMLRocks renders it as its hashable counterpart (a sequence
# becomes a tuple, a mapping a tuple of pairs). Annotated mode previously did not apply
# that conversion and raised an opaque `TypeError` ("cannot use
# 'YAMLRocksAnnotatedDict' as a dict key"); it now matches the fast path, including
# when the offending mapping is reached through an include.
_COMPLEX_KEY = b"[1, 2]: value\n"


@pytest.mark.parametrize(
    ("directive", "make"),
    [
        ("k: !include keyed.yaml", None),
        ("k: !include_dir_merge_named d", "dir"),
    ],
)
def test_complex_key_in_included_file_converts(tmp_path, directive, make):
    """A collection-as-mapping-key inside an included file converts to a hashable
    key under annotated mode instead of raising an opaque TypeError. Both a plain
    !include and a merge-named directory yield the same merged mapping."""
    opt = yamlrocks.OPT_ANNOTATED | yamlrocks.OPT_INCLUDES
    if make == "dir":
        (tmp_path / "d").mkdir()
        (tmp_path / "d" / "keyed.yaml").write_bytes(_COMPLEX_KEY)
    else:
        (tmp_path / "keyed.yaml").write_bytes(_COMPLEX_KEY)
    (tmp_path / "c.yaml").write_text(directive + "\n")

    data = yamlrocks.load(str(tmp_path / "c.yaml"), option=opt)
    assert data["k"] == {(1, 2): "value"}


def test_complex_key_agrees_between_fast_and_annotated_via_include(tmp_path):
    """The hashable key from an included file is identical with or without
    OPT_ANNOTATED, so annotated mode does not diverge from the fast path."""
    (tmp_path / "keyed.yaml").write_bytes(b"? {a: 1}\n: value\n")
    (tmp_path / "c.yaml").write_text("k: !include keyed.yaml\n")
    fast = yamlrocks.load(str(tmp_path / "c.yaml"), option=yamlrocks.OPT_INCLUDES)
    annotated = yamlrocks.load(
        str(tmp_path / "c.yaml"),
        option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_ANNOTATED,
    )
    assert fast == annotated == {"k": {(("a", 1),): "value"}}


def test_reject_complex_keys_fires_inside_an_included_file(tmp_path):
    """With OPT_REJECT_COMPLEX_KEYS, a complex key in an included file raises a
    located YAMLRocksComplexKeyError pointing at that file."""
    (tmp_path / "bad.yaml").write_bytes(b"{a: 1}: b\n")
    (tmp_path / "c.yaml").write_text("k: !include bad.yaml\n")
    opt = (
        yamlrocks.OPT_ANNOTATED
        | yamlrocks.OPT_INCLUDES
        | yamlrocks.OPT_REJECT_COMPLEX_KEYS
    )
    with pytest.raises(yamlrocks.YAMLRocksComplexKeyError) as err:
        yamlrocks.load(str(tmp_path / "c.yaml"), option=opt)
    assert err.value.file.endswith("bad.yaml")
