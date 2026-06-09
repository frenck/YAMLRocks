"""YAMLRocksDocument.save() - write back only the changed files (HA workflow)."""

from __future__ import annotations

import pytest

import yamlrocks

RT_INC = yamlrocks.OPT_ROUND_TRIP | yamlrocks.OPT_INCLUDES


@pytest.fixture
def config(tmp_path):
    (tmp_path / "automations.yaml").write_text(
        "# My automations\n- alias: Morning\n  trigger: sunrise\n"
    )
    (tmp_path / "secrets.yaml").write_text("token: ABC\n")
    root = tmp_path / "configuration.yaml"
    root.write_text(
        "# HA config\n"
        "automation: !include automations.yaml\n"
        "name: Home\n"
        "api: !secret token\n"
    )
    return tmp_path, root


def test_origin_is_tracked(config):
    """A loaded YAMLRocksDocument records the file path it came from as origin."""
    _tmp_path, root = config
    doc = yamlrocks.load(root, option=RT_INC)
    assert doc.origin == str(root)


def test_save_writes_only_changed_include(config):
    """save() rewrites only the included file that was edited."""
    tmp_path, root = config
    original_root = root.read_text()
    doc = yamlrocks.load(root, option=RT_INC)
    doc["automation"][0]["trigger"] = "civil_twilight"

    written = doc.save()
    assert [p.split("/")[-1] for p in written] == ["automations.yaml"]
    assert "civil_twilight" in (tmp_path / "automations.yaml").read_text()
    # The root file is left exactly as it was.
    assert root.read_text() == original_root


def test_save_writes_root_on_root_edit(config):
    """save() rewrites the root file and leaves includes and secrets intact."""
    tmp_path, root = config
    automations_before = (tmp_path / "automations.yaml").read_text()
    doc = yamlrocks.load(root, option=RT_INC)
    doc["name"] = "My House"

    written = doc.save()
    assert [p.split("/")[-1] for p in written] == ["configuration.yaml"]
    assert "My House" in root.read_text()
    # Includes untouched, and the secret is not expanded into the root.
    assert (tmp_path / "automations.yaml").read_text() == automations_before
    assert "ABC" not in root.read_text()
    assert "!secret token" in root.read_text()


def test_save_nothing_when_unmodified(config):
    """save() writes nothing and returns an empty list when unmodified."""
    _, root = config
    doc = yamlrocks.load(root, option=RT_INC)
    assert doc.save() == []


def test_save_as_to_explicit_path(tmp_path):
    """save(path) writes to the given path byte-for-byte when unmodified."""
    src = tmp_path / "in.yaml"
    src.write_text("a: 1\nb: 2\n")
    doc = yamlrocks.load(src, option=yamlrocks.OPT_ROUND_TRIP)
    out = tmp_path / "out.yaml"
    written = doc.save(str(out))
    assert written == [str(out)]
    # Save-as of an unmodified document is byte-for-byte.
    assert out.read_bytes() == src.read_bytes()


def test_save_without_origin_raises(tmp_path):
    """save() raises when the YAMLRocksDocument was not loaded from a file."""
    doc = yamlrocks.loads(b"a: 1\n", option=yamlrocks.OPT_ROUND_TRIP)
    doc["a"] = 2
    with pytest.raises(ValueError, match="not loaded from a file"):
        doc.save()


def test_dump_document_no_target_saves(config):
    """dump(document) without a target saves the document in place."""
    tmp_path, root = config
    doc = yamlrocks.load(root, option=RT_INC)
    doc["automation"][0]["trigger"] = "noon"
    yamlrocks.dump(doc)
    assert "noon" in (tmp_path / "automations.yaml").read_text()


def test_dump_no_target_non_document_raises():
    """dump() without a target raises for a non-YAMLRocksDocument object."""
    with pytest.raises(TypeError):
        yamlrocks.dump({"a": 1})
