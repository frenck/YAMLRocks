"""JSON Schema validation via the ``schema=`` argument to ``loads``."""

from __future__ import annotations

import pytest

import yamlrocks

SCHEMA = {
    "type": "object",
    "required": ["name", "port"],
    "properties": {
        "name": {"type": "string", "minLength": 1},
        "port": {"type": "integer", "minimum": 1, "maximum": 65535},
        "tags": {"type": "array", "items": {"type": "string"}, "minItems": 1},
        "mode": {"enum": ["dev", "prod"]},
    },
    "additionalProperties": False,
}


def test_valid_document_passes():
    """A document matching the schema loads successfully."""
    data = yamlrocks.loads(b"name: app\nport: 8080\ntags:\n  - web\n", schema=SCHEMA)
    assert data == {"name": "app", "port": 8080, "tags": ["web"]}


def test_missing_required_property():
    """A missing required property raises YAMLRocksDecodeError."""
    with pytest.raises(
        yamlrocks.YAMLRocksDecodeError, match="required property 'port'"
    ):
        yamlrocks.loads(b"name: app\n", schema=SCHEMA)


def test_type_mismatch_reports_location():
    """A type mismatch reports the offending path and line."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match=r"\$\.port .*line 2"):
        yamlrocks.loads(b"name: app\nport: not-a-number\n", schema=SCHEMA)


def test_minimum_violation():
    """A value exceeding the maximum raises YAMLRocksDecodeError."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="greater than maximum"):
        yamlrocks.loads(b"name: app\nport: 70000\n", schema=SCHEMA)


def test_min_length_violation():
    """A string shorter than minLength raises YAMLRocksDecodeError."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="minLength"):
        yamlrocks.loads(b"name: ''\nport: 80\n", schema=SCHEMA)


def test_array_item_type():
    """A wrongly-typed array item reports its indexed path."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match=r"\$\.tags\[1\]"):
        yamlrocks.loads(b"name: app\nport: 80\ntags:\n  - ok\n  - 5\n", schema=SCHEMA)


def test_enum_violation():
    """A value outside the enum raises YAMLRocksDecodeError."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="enum"):
        yamlrocks.loads(b"name: app\nport: 80\nmode: staging\n", schema=SCHEMA)


def test_const_object_compares_full_structure():
    """`const` on an object matches by full contents, not as an empty mapping.

    The resolver previously collapsed every mapping to empty, so `const {}`
    accepted any mapping and a non-empty `const` accepted none.
    """
    schema = {"properties": {"a": {"const": {"k": 1}}}}
    # An equal object passes; order does not matter.
    assert yamlrocks.loads(b"a:\n  k: 1\n", schema=schema) == {"a": {"k": 1}}
    assert yamlrocks.loads(
        b"a:\n  k: 1\n  j: 2\n",
        schema={"properties": {"a": {"const": {"j": 2, "k": 1}}}},
    ) == {"a": {"k": 1, "j": 2}}
    # A different value, and a non-empty object against `const {}`, both fail.
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="const"):
        yamlrocks.loads(b"a:\n  k: 2\n", schema=schema)
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="const"):
        yamlrocks.loads(b"a:\n  k: 1\n", schema={"properties": {"a": {"const": {}}}})


def test_enum_array_compares_full_structure():
    """`enum` options that are arrays match by full contents."""
    schema = {"properties": {"a": {"enum": [[1, 2], [3, 4]]}}}
    assert yamlrocks.loads(b"a: [3, 4]\n", schema=schema) == {"a": [3, 4]}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="enum"):
        yamlrocks.loads(b"a: [1, 3]\n", schema=schema)
    # An empty-array enum option must not match a non-empty array.
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="enum"):
        yamlrocks.loads(b"a: [1]\n", schema={"properties": {"a": {"enum": [[]]}}})


def test_ref_resolves_against_defs():
    """A `$ref` to `#/$defs/...` validates against the referenced subschema."""
    schema = {"$ref": "#/$defs/port", "$defs": {"port": {"type": "integer"}}}
    assert yamlrocks.loads(b"42", schema=schema) == 42
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="integer"):
        yamlrocks.loads(b"not_int", schema=schema)


def test_ref_resolves_against_definitions():
    """A `$ref` to the draft-7 `#/definitions/...` location works too."""
    schema = {
        "properties": {"p": {"$ref": "#/definitions/p"}},
        "definitions": {"p": {"type": "string"}},
    }
    assert yamlrocks.loads(b"p: hello\n", schema=schema) == {"p": "hello"}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="string"):
        yamlrocks.loads(b"p: 5\n", schema=schema)


def test_ref_resolves_a_deep_json_pointer():
    """A `$ref` may point at any path within the schema, not just $defs."""
    schema = {
        "$ref": "#/$defs/a/properties/b",
        "$defs": {"a": {"properties": {"b": {"type": "boolean"}}}},
    }
    assert yamlrocks.loads(b"true", schema=schema) is True
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="boolean"):
        yamlrocks.loads(b"5", schema=schema)


def test_ref_recurses_over_a_finite_document():
    """A recursive schema validates a finite nested document (no false cycle)."""
    schema = {
        "$ref": "#/$defs/node",
        "$defs": {
            "node": {
                "type": "object",
                "properties": {"child": {"$ref": "#/$defs/node"}},
            }
        },
    }
    assert yamlrocks.loads(b"child:\n  child:\n    x: 1\n", schema=schema) == {
        "child": {"child": {"x": 1}}
    }


def test_unresolvable_ref_is_an_error():
    """An unresolvable `$ref` is reported, never silently treated as permissive."""
    # A missing local pointer.
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="unresolvable schema"):
        yamlrocks.loads(b"1", schema={"$ref": "#/$defs/nope", "$defs": {}})
    # A remote reference, which would need an external resolver.
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="unresolvable schema"):
        yamlrocks.loads(b"1", schema={"$ref": "https://example.com/s.json"})


def test_cyclic_ref_is_bounded():
    """A `$ref` cycle is cut by the depth bound rather than looping forever."""
    schema = {"$ref": "#/$defs/a", "$defs": {"a": {"$ref": "#/$defs/a"}}}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="nesting too deep"):
        yamlrocks.loads(b"1", schema=schema)


def test_branching_ref_cycle_is_bounded():
    """A `$ref` cycle that branches through a combinator is bounded by a budget.

    `allOf` reaches the same `$ref` twice, doubling the work at each level; a
    per-chain depth cap alone would allow ~2^depth calls (an effective hang). A
    shared total-follow budget cuts it.
    """
    inner = {"allOf": [{"$ref": "#/$defs/a"}, {"$ref": "#/$defs/a"}]}
    schema = {"$ref": "#/$defs/a", "$defs": {"a": inner}}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="nesting too deep"):
        yamlrocks.loads(b"1", schema=schema)


def test_deeply_nested_value_under_enum_does_not_overflow():
    """A deep document under a container enum/const tears down without crashing.

    `enum`/`const` build a fully resolved copy of the value; that deep tree must
    be dropped iteratively, or a near-MAX_DEPTH document could overflow the stack
    on teardown (and abort under `panic = "abort"`).
    """
    depth = 900
    doc = (b"[" * depth) + b"1" + (b"]" * depth)
    # The exact verdict does not matter; the point is no crash on teardown.
    with pytest.raises(yamlrocks.YAMLRocksDecodeError):
        yamlrocks.loads(doc, schema={"enum": [[]]})
    with pytest.raises(yamlrocks.YAMLRocksDecodeError):
        yamlrocks.loads(doc, schema={"const": []})


def test_additional_property_rejected():
    """An undeclared property is rejected when additionalProperties is false."""
    with pytest.raises(
        yamlrocks.YAMLRocksDecodeError, match="additional property 'extra'"
    ):
        yamlrocks.loads(b"name: app\nport: 80\nextra: 1\n", schema=SCHEMA)


def test_min_items_violation():
    """An array shorter than minItems raises YAMLRocksDecodeError."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="minItems"):
        yamlrocks.loads(b"name: app\nport: 80\ntags: []\n", schema=SCHEMA)


def test_nested_schema():
    """Validation recurses into nested object schemas."""
    schema = {
        "type": "object",
        "properties": {
            "server": {
                "type": "object",
                "required": ["host"],
                "properties": {"host": {"type": "string"}},
            }
        },
    }
    assert yamlrocks.loads(b"server:\n  host: localhost\n", schema=schema) == {
        "server": {"host": "localhost"}
    }
    with pytest.raises(
        yamlrocks.YAMLRocksDecodeError, match="required property 'host'"
    ):
        yamlrocks.loads(b"server:\n  port: 5432\n", schema=schema)


def test_combinator_any_of():
    """An anyOf combinator accepts matching values and rejects others."""
    schema = {"anyOf": [{"type": "string"}, {"type": "integer"}]}
    assert yamlrocks.loads(b"42", schema=schema) == 42
    assert yamlrocks.loads(b"hello", schema=schema) == "hello"
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="anyOf"):
        yamlrocks.loads(b"3.14", schema=schema)


def test_number_accepts_int_and_float():
    """The number type accepts both int and float values."""
    schema = {"type": "object", "properties": {"x": {"type": "number"}}}
    assert yamlrocks.loads(b"x: 1\n", schema=schema) == {"x": 1}
    assert yamlrocks.loads(b"x: 1.5\n", schema=schema) == {"x": 1.5}


def test_schema_validates_annotated_mode():
    """Schema validation still runs when annotated mode uses the AST path."""
    with pytest.raises(
        yamlrocks.YAMLRocksDecodeError, match="required property 'port'"
    ):
        yamlrocks.loads(b"name: app\n", option=yamlrocks.OPT_ANNOTATED, schema=SCHEMA)


def test_schema_validates_round_trip_mode():
    """Schema validation still runs when round-trip mode returns a YAMLRocksDocument."""
    with pytest.raises(
        yamlrocks.YAMLRocksDecodeError, match="required property 'port'"
    ):
        yamlrocks.loads(b"name: app\n", option=yamlrocks.OPT_ROUND_TRIP, schema=SCHEMA)


def test_schema_validates_included_nodes(tmp_path):
    """Schema validation sees include-resolved values on the AST path."""
    (tmp_path / "port.yaml").write_text("not-a-number\n")
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match=r"\$\.port"):
        yamlrocks.loads(
            b"name: app\nport: !include port.yaml\n",
            option=yamlrocks.OPT_INCLUDES,
            include_dir=tmp_path,
            schema=SCHEMA,
        )


def test_schema_honors_yaml_1_1_resolution():
    """Schema validation uses YAML 1.1 scalar rules when requested."""
    schema = {"type": "object", "properties": {"flag": {"type": "boolean"}}}
    assert yamlrocks.loads(
        b"flag: yes\n", option=yamlrocks.OPT_YAML_1_1, schema=schema
    ) == {"flag": True}


def test_schema_validates_alias_as_referenced_value():
    """An aliased node validates as the value its anchor refers to."""
    schema = {
        "type": "object",
        "properties": {"b": {"type": "string", "enum": ["hello"]}},
    }
    assert yamlrocks.loads("a: &x hello\nb: *x\n", schema=schema) == {
        "a": "hello",
        "b": "hello",
    }


def test_schema_rejects_alias_violating_type():
    """An aliased value is validated, not silently treated as null."""
    schema = {"type": "object", "properties": {"b": {"type": "integer"}}}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError):
        yamlrocks.loads("a: &x hello\nb: *x\n", schema=schema)


# -- In-file yaml-language-server schema directive ---------------------------

DIRECTIVE = b"# yaml-language-server: $schema=https://example.com/schema.json\n"


def test_schema_ref_detects_first_line_directive():
    """schema_ref returns the declared reference from the leading comment."""
    assert (
        yamlrocks.schema_ref(DIRECTIVE + b"name: app\n")
        == "https://example.com/schema.json"
    )


def test_schema_ref_returns_none_without_directive():
    """schema_ref returns None when no directive is present."""
    assert yamlrocks.schema_ref(b"name: app\n") is None


def test_schema_ref_ignores_plain_comments():
    """A plain comment is not mistaken for a schema directive."""
    assert yamlrocks.schema_ref(b"# just a note\nname: app\n") is None


def test_schema_ref_allows_leading_blank_and_comment_lines():
    """schema_ref scans the whole leading comment block, not only line one."""
    data = b"\n# copyright\n" + DIRECTIVE + b"name: app\n"
    assert yamlrocks.schema_ref(data) == "https://example.com/schema.json"


def test_schema_ref_stops_at_document_body():
    """A directive appearing after content is not detected."""
    assert yamlrocks.schema_ref(b"name: app\n" + DIRECTIVE) is None


def test_schema_ref_accepts_str_input():
    """schema_ref accepts str input as well as bytes."""
    assert (
        yamlrocks.schema_ref("# yaml-language-server: $schema=urn:x\na: 1\n") == "urn:x"
    )


def test_schema_auto_resolves_and_validates():
    """schema='auto' resolves the in-file ref and validates against it."""
    schema = {"type": "object", "properties": {"port": {"type": "integer"}}}
    seen = []

    def resolver(ref):
        seen.append(ref)
        return schema

    data = DIRECTIVE + b"port: not-a-number\n"
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match=r"\$\.port"):
        yamlrocks.loads(data, schema="auto", schema_resolver=resolver)
    assert seen == ["https://example.com/schema.json"]


def test_schema_auto_passes_for_valid_document():
    """schema='auto' returns the parsed value when the document conforms."""
    schema = {"type": "object", "properties": {"port": {"type": "integer"}}}
    data = DIRECTIVE + b"port: 8080\n"
    assert yamlrocks.loads(data, schema="auto", schema_resolver=lambda ref: schema) == {
        "port": 8080
    }


def test_schema_auto_skips_when_no_directive():
    """schema='auto' skips validation when the document has no directive."""
    called = False

    def resolver(ref):
        nonlocal called
        called = True
        return {"type": "integer"}

    assert yamlrocks.loads(
        b"port: text\n", schema="auto", schema_resolver=resolver
    ) == {"port": "text"}
    assert called is False


def test_schema_auto_skips_when_resolver_returns_none():
    """schema='auto' skips validation when the resolver declines the ref."""
    data = DIRECTIVE + b"port: not-a-number\n"
    assert yamlrocks.loads(data, schema="auto", schema_resolver=lambda ref: None) == {
        "port": "not-a-number"
    }


def test_schema_auto_requires_resolver():
    """schema='auto' without a resolver is a usage error."""
    with pytest.raises(ValueError, match="schema_resolver"):
        yamlrocks.loads(DIRECTIVE + b"port: 1\n", schema="auto")


def test_schema_resolver_requires_auto():
    """A resolver without schema='auto' is a usage error."""
    with pytest.raises(ValueError, match='schema="auto"'):
        yamlrocks.loads(
            DIRECTIVE + b"port: 1\n", schema_resolver=lambda ref: {"type": "object"}
        )


def test_schema_auto_propagates_resolver_error():
    """An exception raised by the resolver surfaces to the caller."""

    def resolver(ref):
        raise RuntimeError("offline")

    with pytest.raises(RuntimeError, match="offline"):
        yamlrocks.loads(
            DIRECTIVE + b"port: 1\n", schema="auto", schema_resolver=resolver
        )


def test_schema_dict_path_still_works():
    """The explicit schema dict path is unchanged by the directive feature."""
    schema = {"type": "object", "required": ["port"]}
    with pytest.raises(
        yamlrocks.YAMLRocksDecodeError, match="required property 'port'"
    ):
        yamlrocks.loads(b"name: app\n", schema=schema)


# -- Additional validation keyword and combinator branches -------------------


def test_const_violation():
    """A value not equal to const raises with the const message."""
    schema = {"type": "object", "properties": {"x": {"const": 7}}}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="required const"):
        yamlrocks.loads(b"x: 5\n", schema=schema)


def test_const_passes_for_match():
    """A value equal to const validates successfully."""
    schema = {"type": "object", "properties": {"x": {"const": 7}}}
    assert yamlrocks.loads(b"x: 7\n", schema=schema) == {"x": 7}


def test_minimum_below_bound():
    """A value below minimum reports the minimum message."""
    schema = {"type": "object", "properties": {"x": {"minimum": 1}}}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="less than minimum"):
        yamlrocks.loads(b"x: 0\n", schema=schema)


def test_exclusive_minimum_violation():
    """A value equal to exclusiveMinimum is rejected."""
    schema = {"type": "object", "properties": {"x": {"exclusiveMinimum": 1}}}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="not greater than"):
        yamlrocks.loads(b"x: 1\n", schema=schema)


def test_exclusive_maximum_violation():
    """A value equal to exclusiveMaximum is rejected."""
    schema = {"type": "object", "properties": {"x": {"exclusiveMaximum": 5}}}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="not less than"):
        yamlrocks.loads(b"x: 5\n", schema=schema)


def test_max_length_violation():
    """A string longer than maxLength is rejected."""
    schema = {"type": "object", "properties": {"x": {"maxLength": 3}}}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="maxLength"):
        yamlrocks.loads(b"x: hello\n", schema=schema)


def test_max_items_violation():
    """An array longer than maxItems is rejected."""
    schema = {"type": "object", "properties": {"x": {"maxItems": 2}}}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="maxItems"):
        yamlrocks.loads(b"x: [1, 2, 3]\n", schema=schema)


def test_type_as_list_accepts_any_listed():
    """A type given as a list accepts any of the listed types."""
    schema = {"type": "object", "properties": {"x": {"type": ["string", "integer"]}}}
    assert yamlrocks.loads(b"x: 5\n", schema=schema) == {"x": 5}


def test_type_as_list_rejects_unlisted():
    """A type given as a list rejects a value of an unlisted type."""
    schema = {"type": "object", "properties": {"x": {"type": ["string", "integer"]}}}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match=r"string \| integer"):
        yamlrocks.loads(b"x: 1.5\n", schema=schema)


def test_boolean_false_schema_rejects_everything():
    """A boolean schema of false rejects any value."""
    schema = {"type": "object", "properties": {"x": False}}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="schema is false"):
        yamlrocks.loads(b"x: 1\n", schema=schema)


def test_boolean_true_schema_accepts_everything():
    """A boolean schema of true accepts any value."""
    schema = {"type": "object", "properties": {"x": True}}
    assert yamlrocks.loads(b"x: anything\n", schema=schema) == {"x": "anything"}


def test_additional_properties_subschema_validates():
    """An additionalProperties subschema validates undeclared properties."""
    schema = {"type": "object", "additionalProperties": {"type": "string"}}
    assert yamlrocks.loads(b"a: hello\n", schema=schema) == {"a": "hello"}
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="expected type string"):
        yamlrocks.loads(b"a: 1\n", schema=schema)


def test_all_of_combinator():
    """An allOf combinator enforces every subschema."""
    schema = {"allOf": [{"type": "integer"}, {"minimum": 10}]}
    assert yamlrocks.loads(b"42", schema=schema) == 42
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="less than minimum"):
        yamlrocks.loads(b"5", schema=schema)


def test_one_of_combinator():
    """A oneOf combinator requires exactly one subschema to match."""
    schema = {"oneOf": [{"type": "string"}, {"type": "integer"}]}
    assert yamlrocks.loads(b"hello", schema=schema) == "hello"
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="oneOf"):
        yamlrocks.loads(
            b"5", schema={"oneOf": [{"type": "integer"}, {"type": "number"}]}
        )


def test_not_combinator():
    """A not combinator rejects values matching its subschema."""
    schema = {"not": {"type": "integer"}}
    assert yamlrocks.loads(b"hello", schema=schema) == "hello"
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="must not match"):
        yamlrocks.loads(b"5", schema=schema)


def test_non_object_schema_is_permissive():
    """A schema that is neither a boolean nor an object accepts anything."""
    assert yamlrocks.loads(b"x: 1\n", schema=[1, 2, 3]) == {"x": 1}


def test_schema_bigint_is_integer_and_bounded():
    """An integer beyond i64 (Value::BigInt) still validates as type integer and
    is checked against numeric bounds."""
    big = b"n: 100000000000000000000000000\n"
    assert (
        yamlrocks.loads(
            big, schema={"type": "object", "properties": {"n": {"type": "integer"}}}
        )
        is not None
    )
    with pytest.raises(yamlrocks.YAMLRocksSchemaError):
        yamlrocks.loads(
            big,
            schema={
                "type": "object",
                "properties": {"n": {"type": "integer", "maximum": 100}},
            },
        )


def test_schema_accepts_integer_valued_float_bounds():
    """A count bound written as an integer-valued float (minItems: 2.0) is
    honored, as JSON Schema permits either spelling."""
    schema = {"type": "object", "properties": {"a": {"type": "array", "minItems": 2.0}}}
    assert yamlrocks.loads(b"a: [1, 2]\n", schema=schema) == {"a": [1, 2]}
    with pytest.raises(yamlrocks.YAMLRocksSchemaError):
        yamlrocks.loads(b"a: [1]\n", schema=schema)
