mod decode;
mod emit_util;
mod encode;
mod ffi;
mod include;
mod parser;
mod resolver;
mod roundtrip;
mod scanner;
mod schema;
mod stack;
mod typeref;
mod version;

/// Entry points for fuzz harnesses (see `fuzz/`). Hidden from the public surface
/// and present only so a libFuzzer target can drive the pure-Rust engine without
/// going through the Python layer. Not part of any stable API.
#[doc(hidden)]
pub mod fuzz {
    use crate::decode::decode_with;
    use crate::encode::{encode, EmitOptions, NullStyle};
    use crate::resolver::Schema;
    use crate::roundtrip::composer::compose;
    use crate::roundtrip::emit::emit_roundtrip_all_with;

    /// Run arbitrary text through the full scanner → parser → composer pipeline.
    /// It must never panic or hang: it returns an AST or a clean parse error.
    pub fn parse(input: &str) {
        let _ = compose(input);
    }

    /// Drive the fast `loads` decode path (events → `Value` tree) under both the
    /// YAML 1.2 and 1.1 schemas, then re-emit each document through the fast
    /// emitter. This covers code distinct from [`parse`]: direct decoding, merge
    /// keys, scalar resolution, and the fast `dumps` emitter, the path most
    /// callers actually hit. The contract is the same, never panic on any input.
    pub fn decode(input: &str) {
        for schema in [Schema::Yaml12, Schema::Yaml11] {
            if let Ok(documents) = decode_with(input, schema, false, false) {
                let options = EmitOptions::default();
                for document in &documents {
                    let _ = encode(document, &options);
                }
            }
        }
    }

    /// Round-trip arbitrary text through compose → emit → re-compose, exercising
    /// the round-trip emitter (the code behind `to_yaml`/`save`) that the other
    /// targets do not reach. The contract is the never-panic/never-hang/never-UB
    /// one, the same as [`parse`] and [`decode`].
    ///
    /// It deliberately does *not* assert that the re-emitted output re-parses.
    /// Re-emitting a *modified* document can, for inputs that never occur in
    /// real YAML (CR line breaks, a bare U+FEFF, a tagged null key following a
    /// null value), produce output the composer rejects. Byte-for-byte fidelity
    /// for realistic documents is enforced instead by the YAML test suite and
    /// the real-world corpus; tightening this into a strict re-parse invariant
    /// is a tracked stretch goal once the emitter handles that exotic tail. The
    /// output is still fed back through the composer so a panic anywhere in that
    /// path is caught.
    pub fn roundtrip(input: &str) {
        let Ok(nodes) = compose(input) else { return };
        let emitted = emit_roundtrip_all_with(&nodes, NullStyle::default());
        if let Ok(text) = std::str::from_utf8(&emitted) {
            let _ = compose(text);
        }
    }

    /// Differential: the fast decoder and the fast emitter must agree on the
    /// data. Decoding `input` yields a `Value` tree; emitting that tree with
    /// `dumps` and decoding the result again must yield an *equal* tree. A
    /// divergence means `dumps` produced YAML that `loads` reads back as
    /// different data, the silent-corruption bug class that matters most for a
    /// YAML library and the one a never-panic target cannot catch (no crash,
    /// just wrong values).
    ///
    /// Restricted to the YAML 1.2 schema on purpose: the fast emitter targets
    /// 1.2 quoting rules, so re-decoding its output under 1.1 (where bare `yes`
    /// is a bool, not the string it emitted) would diverge by design, not by
    /// bug. Every other step is a hard assertion: see [`check_emit_roundtrip`].
    pub fn differential(input: &str) {
        let Ok(documents) = decode_with(input, Schema::Yaml12, false, false) else {
            return;
        };
        let options = EmitOptions::default();
        for document in &documents {
            check_emit_roundtrip(input, document, &options);
        }
    }

    /// Like [`differential`], but emits each document under a matrix of
    /// *non-default* [`EmitOptions`]: flow style, wider indentation, indentless
    /// sequences, the alternate null styles, single-quote preference, explicit
    /// document markers, and several line widths. The emitter must preserve the
    /// decoded data under every presentation choice; width-driven line breaking
    /// (scalar folding, flow-separator breaks) and the indentless layout are the
    /// prime suspects, since each may only break where it cannot change the value.
    ///
    /// `sort_keys` is left out on purpose: it reorders mapping pairs, and the data
    /// comparison is order-sensitive (a YAML mapping is unordered, but the decoded
    /// `Value` keeps source order), so a sort would diverge by design, not by bug.
    pub fn differential_options(input: &str) {
        let Ok(documents) = decode_with(input, Schema::Yaml12, false, false) else {
            return;
        };
        let base = EmitOptions::default();
        let configs = [
            EmitOptions {
                flow_style: true,
                ..base.clone()
            },
            EmitOptions {
                indent: 4,
                ..base.clone()
            },
            EmitOptions {
                indentless_sequences: true,
                ..base.clone()
            },
            EmitOptions {
                null_style: NullStyle::Tilde,
                ..base.clone()
            },
            EmitOptions {
                null_style: NullStyle::Null,
                ..base.clone()
            },
            EmitOptions {
                double_quotes: false,
                ..base.clone()
            },
            EmitOptions {
                explicit_start: true,
                explicit_end: true,
                ..base.clone()
            },
            EmitOptions {
                width: 1,
                ..base.clone()
            },
            EmitOptions {
                width: 20,
                ..base.clone()
            },
            EmitOptions {
                width: 80,
                ..base.clone()
            },
            EmitOptions {
                flow_style: true,
                width: 20,
                ..base.clone()
            },
        ];
        for document in &documents {
            for options in &configs {
                check_emit_roundtrip(input, document, options);
            }
        }
    }

    /// Emit `document` with `options`, re-decode the output, and assert the data
    /// survived unchanged. Shared by both differential targets so a divergence is
    /// reported identically (the panic message names the options that triggered
    /// it).
    ///
    /// Every step is a hard assertion, unlike the round-trip target's lenient
    /// re-compose. The fast emitter takes a freshly decoded `Value` tree (no user
    /// edits) and must always produce valid YAML that `loads` reads back as the
    /// *same single document*. So invalid UTF-8, output that fails to re-decode, or
    /// output that splits into several documents is itself a `dumps` bug, not a
    /// case to skip. Three such bugs were found and fixed this way: an unquoted
    /// `...`-marker string, a folded scalar breaking onto a marker line, and an
    /// indentless tagged sequence item merging into its parent.
    fn check_emit_roundtrip(
        input: &str,
        document: &crate::decode::Value<'_>,
        options: &EmitOptions,
    ) {
        let emitted = encode(document, options);
        let text = std::str::from_utf8(&emitted).unwrap_or_else(|e| {
            panic!("dumps emitted invalid UTF-8\n  options:  {options:?}\n  input:    {input:?}\n  error:    {e}")
        });
        let reparsed = decode_with(text, Schema::Yaml12, false, false).unwrap_or_else(|e| {
            panic!("dumps emitted YAML that loads rejects\n  options:  {options:?}\n  input:    {input:?}\n  emitted:  {text:?}\n  error:    {e:?}")
        });
        assert_eq!(
            reparsed.len(),
            1,
            "dumps emitted multi-document YAML\n  options:  {options:?}\n  input:    {input:?}\n  emitted:  {text:?}"
        );
        assert!(
            values_equiv(document, &reparsed[0]),
            "dumps/loads diverged on data\n  options:  {options:?}\n  input:    {input:?}\n  emitted:  {text:?}\n  original: {document:?}\n  reparsed: {:?}",
            reparsed[0]
        );
    }

    /// Structural equality of two decoded `Value` trees, identical to the
    /// derived `PartialEq` except that two NaN floats compare equal. NaN arises
    /// from `.nan` and never round-trips under `==` (`NaN != NaN`), so without
    /// this every NaN-bearing document would be a spurious differential finding.
    fn values_equiv(a: &crate::decode::Value<'_>, b: &crate::decode::Value<'_>) -> bool {
        use crate::decode::Value::{
            BigInt, Bool, Float, Int, Mapping, Null, Sequence, String as Str, Tagged,
        };
        match (a, b) {
            (Null, Null) => true,
            (Bool(x), Bool(y)) => x == y,
            (Int(x), Int(y)) => x == y,
            (BigInt(x), BigInt(y)) => x == y,
            // f64 `==` already treats +0.0 and -0.0 as equal (the right call: a
            // sign flip on zero is not data loss); only NaN needs special care.
            (Float(x), Float(y)) => x == y || (x.is_nan() && y.is_nan()),
            (Str(x), Str(y)) => x == y,
            (Sequence(x), Sequence(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(p, q)| values_equiv(p, q))
            }
            (Mapping(x), Mapping(y)) => {
                x.len() == y.len()
                    && x.iter()
                        .zip(y)
                        .all(|((pk, pv), (qk, qv))| values_equiv(pk, qk) && values_equiv(pv, qv))
            }
            (Tagged(tx, vx), Tagged(ty, vy)) => tx == ty && values_equiv(vx, vy),
            _ => false,
        }
    }
}

use pyo3::prelude::*;

// The module holds no mutable global state (all state lives in per-instance
// pyclasses guarded by PyO3), so it is safe to run without the GIL on
// free-threaded (nogil) CPython builds.
#[pymodule(gil_used = false)]
fn _yamlrocks(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(ffi::loads, m)?)?;
    m.add_function(wrap_pyfunction!(ffi::loads_all, m)?)?;
    m.add_function(wrap_pyfunction!(ffi::dumps, m)?)?;
    m.add_function(wrap_pyfunction!(ffi::to_json, m)?)?;
    m.add_function(wrap_pyfunction!(ffi::schema_ref, m)?)?;
    m.add_function(wrap_pyfunction!(ffi::yaml_version, m)?)?;
    m.add_function(wrap_pyfunction!(ffi::dump_includes, m)?)?;
    m.add_function(wrap_pyfunction!(ffi::dump_includes_map, m)?)?;

    // Registered in the same grouped order as their definitions in `ffi`.
    // Reading: schema and YAML version.
    m.add("OPT_YAML_1_1", ffi::OPT_YAML_1_1)?;
    m.add("OPT_PYYAML_COMPAT", ffi::OPT_PYYAML_COMPAT)?;
    m.add("OPT_UPGRADE_1_1", ffi::OPT_UPGRADE_1_1)?;
    m.add("OPT_YAML_1_1_WARN", ffi::OPT_YAML_1_1_WARN)?;
    // Reading: result shape.
    m.add("OPT_ROUND_TRIP", ffi::OPT_ROUND_TRIP)?;
    m.add("OPT_ANNOTATED", ffi::OPT_ANNOTATED)?;
    m.add("OPT_ANNOTATE_NUMBERS", ffi::OPT_ANNOTATE_NUMBERS)?;
    // Reading: includes.
    m.add("OPT_INCLUDES", ffi::OPT_INCLUDES)?;
    m.add("OPT_INCLUDE_DIR_RECURSIVE", ffi::OPT_INCLUDE_DIR_RECURSIVE)?;
    // Reading: config tags (secrets and environment variables).
    m.add("OPT_SECRETS", ffi::OPT_SECRETS)?;
    m.add("OPT_ENV_VAR", ffi::OPT_ENV_VAR)?;
    m.add("OPT_SECRET_NOT_FOUND_WARN", ffi::OPT_SECRET_NOT_FOUND_WARN)?;
    m.add(
        "OPT_ENV_VAR_NOT_FOUND_WARN",
        ffi::OPT_ENV_VAR_NOT_FOUND_WARN,
    )?;
    // Reading: tags and keys.
    m.add("OPT_PASSTHROUGH_TAG", ffi::OPT_PASSTHROUGH_TAG)?;
    m.add("OPT_DUPLICATE_KEYS_ERROR", ffi::OPT_DUPLICATE_KEYS_ERROR)?;
    m.add("OPT_DUPLICATE_KEYS_WARN", ffi::OPT_DUPLICATE_KEYS_WARN)?;
    m.add("OPT_REJECT_COMPLEX_KEYS", ffi::OPT_REJECT_COMPLEX_KEYS)?;
    // Writing: layout.
    m.add("OPT_INDENT_2", ffi::OPT_INDENT_2)?;
    m.add("OPT_INDENT_4", ffi::OPT_INDENT_4)?;
    m.add("OPT_INDENTLESS_SEQUENCES", ffi::OPT_INDENTLESS_SEQUENCES)?;
    m.add("OPT_FLOW_STYLE", ffi::OPT_FLOW_STYLE)?;
    m.add("OPT_SORT_KEYS", ffi::OPT_SORT_KEYS)?;
    m.add("OPT_EXPLICIT_START", ffi::OPT_EXPLICIT_START)?;
    m.add("OPT_EXPLICIT_END", ffi::OPT_EXPLICIT_END)?;
    // Writing: scalar style.
    m.add("OPT_SINGLE_QUOTES", ffi::OPT_SINGLE_QUOTES)?;
    m.add("OPT_NULL_AS_KEYWORD", ffi::OPT_NULL_AS_KEYWORD)?;
    m.add("OPT_NULL_AS_TILDE", ffi::OPT_NULL_AS_TILDE)?;
    // Writing: type serialization.
    m.add("OPT_SERIALIZE_NUMPY", ffi::OPT_SERIALIZE_NUMPY)?;
    m.add("OPT_PASSTHROUGH_DATETIME", ffi::OPT_PASSTHROUGH_DATETIME)?;
    m.add("OPT_PASSTHROUGH_DATACLASS", ffi::OPT_PASSTHROUGH_DATACLASS)?;
    m.add("OPT_OMIT_MICROSECONDS", ffi::OPT_OMIT_MICROSECONDS)?;
    m.add("OPT_NAIVE_UTC", ffi::OPT_NAIVE_UTC)?;
    m.add("OPT_UTC_Z", ffi::OPT_UTC_Z)?;

    // The exception hierarchy is defined in pure Python (yamlrocks.exceptions) and
    // raised from Rust via pyo3::import_exception!; it is not registered here.
    m.add_class::<roundtrip::document::YAMLRocksDocument>()?;
    m.add_class::<roundtrip::document::YAMLRocksDocumentView>()?;
    m.add_class::<roundtrip::document::YAMLRocksNode>()?;
    m.add_class::<ffi::YAMLRocksTag>()?;
    m.add_class::<ffi::YAMLRocksAnnotatedDict>()?;
    m.add_class::<ffi::YAMLRocksAnnotatedList>()?;

    Ok(())
}
