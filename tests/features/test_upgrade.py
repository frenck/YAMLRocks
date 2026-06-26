"""YAML 1.1 to 1.2 upgrade (``yamlrocks.upgrade`` / ``OPT_UPGRADE_1_1``)."""

from __future__ import annotations

import yamlrocks

# upgrade() stamps a `%YAML 1.2` version directive so the result declares itself
# as upgraded and is read back as 1.2 (not re-coerced under OPT_UPGRADE_1_1).
STAMP = b"%YAML 1.2\n---\n"


def test_upgrade_booleans():
    """upgrade() converts 1.1 yes/no/on/off scalars to true/false."""
    out = yamlrocks.upgrade(b"a: yes\nb: no\nc: on\nd: off\n")
    assert out == STAMP + b"a: true\nb: false\nc: true\nd: false\n"


def test_upgrade_octal():
    """upgrade() converts 1.1 leading-zero octals to decimal integers."""
    assert yamlrocks.upgrade(b"perms: 0777\n") == STAMP + b"perms: 511\n"


def test_upgrade_leaves_12_values_untouched():
    """upgrade() leaves values already valid in 1.2 unchanged (but stamps it)."""
    src = b"a: true\nb: 42\nc: hello\nd: 3.14\n"
    assert yamlrocks.upgrade(src) == STAMP + src


def test_upgrade_preserves_comments():
    """upgrade() keeps header and inline comments while converting values."""
    src = b"# header\nenabled: yes  # inline\nname: app\n"
    out = yamlrocks.upgrade(src).decode()
    assert "# header" in out
    assert "# inline" in out
    assert "enabled: true" in out
    assert "name: app" in out


def test_upgrade_preserves_unchanged_values():
    """upgrade() preserves unchanged values and comments when re-emitting."""
    # Once a scalar is upgraded the document is re-emitted, so spacing may be
    # normalized, but unchanged values and comments are preserved.
    src = b"untouched: spaced_value  # keep me\nflag: yes\n"
    out = yamlrocks.upgrade(src).decode()
    assert "untouched: spaced_value" in out
    assert "# keep me" in out
    assert "flag: true" in out


def test_upgrade_unchanged_document_only_gains_the_stamp():
    """A pure 1.2 document keeps its content verbatim, gaining only the stamp."""
    # No scalar needs changing, so the body is preserved exactly; upgrade only
    # prepends the `%YAML 1.2` declaration.
    src = b"a: 1\nb:    2   # spaced\nc: hello\n"
    assert yamlrocks.upgrade(src) == STAMP + src


def test_upgrade_nested():
    """upgrade() converts 1.1 booleans nested in mappings and sequences."""
    src = b"outer:\n  inner: off\n  list:\n    - yes\n    - no\n"
    out = yamlrocks.upgrade(src)
    assert yamlrocks.loads(out) == {"outer": {"inner": False, "list": [True, False]}}


def test_upgrade_without_comments_reformats():
    """upgrade(preserve_comments=False) still converts 1.1 booleans."""
    out = yamlrocks.upgrade(b"a: yes\nb: no\n", preserve_comments=False)
    assert yamlrocks.loads(out) == {"a": True, "b": False}


def test_upgrade_round_trips_to_valid_12():
    """upgrade() output parses as real booleans under the 1.2 loader."""
    # After upgrade, the default (1.2) loader sees real booleans.
    out = yamlrocks.upgrade(b"enabled: yes\n")
    assert yamlrocks.loads(out) == {"enabled": True}


def test_opt_upgrade_via_loads():
    """OPT_UPGRADE_1_1 applies the 1.1 upgrade during loads()."""
    doc = yamlrocks.loads(
        b"x: yes\n", option=yamlrocks.OPT_ROUND_TRIP | yamlrocks.OPT_UPGRADE_1_1
    )
    assert doc.to_yaml() == STAMP + b"x: true\n"


def test_upgrade_single_letter_booleans():
    """upgrade() converts single-letter 1.1 y/n booleans to true/false."""
    out = yamlrocks.upgrade(b"a: y\nb: n\n")
    assert out == STAMP + b"a: true\nb: false\n"


def test_upgrade_hex_left_unchanged():
    """upgrade() leaves hex integers alone since 1.2 also reads them as ints."""
    # 0xFF is integer 255 in both 1.1 and 1.2, so no rewrite is needed.
    assert yamlrocks.upgrade(b"x: 0xFF\n") == STAMP + b"x: 0xFF\n"


def test_upgrade_binary_to_decimal():
    """upgrade() rewrites binary integers to canonical decimal."""
    assert yamlrocks.upgrade(b"x: 0b1010\n") == STAMP + b"x: 10\n"


def test_upgrade_sexagesimal_to_decimal():
    """upgrade() rewrites a base-60 integer to its decimal value."""
    assert yamlrocks.upgrade(b"x: 1:30\n") == STAMP + b"x: 90\n"


def test_upgrade_quoted_scalar_untouched():
    """upgrade() leaves quoted scalars alone since they are always strings."""
    src = b'x: "yes"\n'
    assert yamlrocks.upgrade(src) == STAMP + src


def test_upgrade_float_gets_canonical_form():
    """upgrade() rewrites a 1.1 sexagesimal float to a canonical 1.2 float."""
    # 1:30.5 is a YAML 1.1 sexagesimal float; 1.2 has no such form.
    out = yamlrocks.upgrade(b"x: 1:30.5\n")
    assert yamlrocks.loads(out) == {"x": 90.5}


# -- The %YAML version directive: detection, honoring, and the upgrade round-trip


def test_yaml_version_detects_declared_version():
    """yaml_version() reports the document's declared %YAML version, or None."""
    assert yamlrocks.yaml_version(b"%YAML 1.1\n---\nx: 1\n") == "1.1"
    assert yamlrocks.yaml_version(b"%YAML 1.2\n---\nx: 1\n") == "1.2"
    assert yamlrocks.yaml_version(b"x: 1\n") is None


def test_directive_overrides_the_flag():
    """A document's %YAML directive selects the schema, overriding the flags."""
    # %YAML 1.2 wins even when the caller asks for 1.1.
    src_12 = b"%YAML 1.2\n---\nx: yes\n"
    assert yamlrocks.loads(src_12, option=yamlrocks.OPT_YAML_1_1) == {"x": "yes"}
    assert yamlrocks.loads(src_12, option=yamlrocks.OPT_UPGRADE_1_1) == {"x": "yes"}
    # %YAML 1.1 wins even in the default 1.2 mode.
    assert yamlrocks.loads(b"%YAML 1.1\n---\nx: yes\n") == {"x": True}


def test_no_directive_keeps_flag_behavior():
    """Without a directive the flags still decide the schema."""
    assert yamlrocks.loads(b"x: yes\n") == {"x": "yes"}
    assert yamlrocks.loads(b"x: yes\n", option=yamlrocks.OPT_YAML_1_1) == {"x": True}


def test_upgrade_stamps_version_directive():
    """upgrade() prepends a %YAML 1.2 directive declaring the result."""
    out = yamlrocks.upgrade(b"enabled: yes\n")
    assert out.startswith(b"%YAML 1.2\n---\n")
    assert yamlrocks.yaml_version(out) == "1.2"


def test_upgrade_is_idempotent():
    """Upgrading an already-stamped document does not add a second directive."""
    once = yamlrocks.upgrade(b"# c\nenabled: yes\nmask: 0777\n")
    assert yamlrocks.upgrade(once) == once


def test_stamped_file_is_safe_under_persistent_upgrade_mode():
    """A stamped file is read as 1.2 under OPT_UPGRADE_1_1, not re-coerced.

    This is the migration guarantee: once a file is upgraded and stamped, a
    later value the user writes that *looks* like a 1.1 boolean stays a string.
    """
    upgraded = yamlrocks.upgrade(b"enabled: yes\n")
    edited = upgraded + b"note: yes\n"
    result = yamlrocks.loads(edited, option=yamlrocks.OPT_UPGRADE_1_1)
    assert result == {"enabled": True, "note": "yes"}


def test_roundtrip_upgrade_save_stamps(tmp_path):
    """A round-trip OPT_UPGRADE_1_1 document stamps the file it writes back."""
    path = tmp_path / "config.yaml"
    path.write_bytes(b"# device\nenabled: yes  # was on\n")
    doc = yamlrocks.load(
        str(path), option=yamlrocks.OPT_ROUND_TRIP | yamlrocks.OPT_UPGRADE_1_1
    )
    doc.save()
    written = path.read_bytes()
    assert written.startswith(b"%YAML 1.2\n---\n")
    # The inline comment keeps its original two-space alignment padding.
    assert b"enabled: true  # was on" in written


def test_upgrade_keeps_tag_directive_in_scope():
    """upgrade() stamps `%YAML 1.2` ahead of a `%TAG`, keeping the handle valid.

    The version directive must precede the `%TAG`, not displace it after a
    document start, or the named handle would no longer resolve on reload.
    """
    src = b"%TAG !e! tag:example.com,2020:\n---\nv: !e!foo old\n"
    out = yamlrocks.upgrade(src)
    assert out == (b"%YAML 1.2\n%TAG !e! tag:example.com,2020:\n---\nv: !e!foo old\n")
    # The result reloads: the handle is still defined for the document.
    assert yamlrocks.loads(out) == {"v": "old"}


def test_upgrade_replaces_declared_version_before_tag_directive():
    """A declared `%YAML 1.1` is replaced by 1.2 while `%TAG` stays in place."""
    src = b"%YAML 1.1\n%TAG !e! tag:x:\n---\nv: !e!foo 1\n"
    out = yamlrocks.upgrade(src)
    assert out == b"%YAML 1.2\n%TAG !e! tag:x:\n---\nv: !e!foo 1\n"
    assert yamlrocks.loads(out) == {"v": "1"}
