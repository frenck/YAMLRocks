mod decode;
mod emit_util;
mod encode;
mod ffi;
mod include;
mod parser;
mod resolver;
mod roundtrip;
mod safe_io;
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
    use crate::parser::ParseOptions;
    use crate::resolver::Schema;
    use crate::roundtrip::ast::{NodeStyle, YamlNode, YamlNodeKind};
    use crate::roundtrip::composer::{compose, compose_stream_with, ComposedStream};
    use crate::roundtrip::emit::{emit_roundtrip_all_with, emit_stream_with};

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

    /// Round-trip differential: re-emitting a round-trip document must never
    /// change its data. The [`roundtrip`] target only proves the emitter does
    /// not crash; this one checks what an edited document's `to_yaml` writes.
    /// For any input both the composer and the fast decoder accept, it forces
    /// the whole stream through the emitter (never the cached source), reloads
    /// the output with the fast decoder, and asserts it holds as many documents
    /// as the input, each with the same data. That is the path every untouched
    /// node takes once anything in its document changes.
    ///
    /// Two more stages mark scalars modified without changing them, as
    /// setting a comment to the one it already has would, and check again. A
    /// modified scalar is rebuilt from its value instead of replayed from its
    /// source, which exercises the emitter's quoting and fallback rules. First
    /// one scalar, picked by a hash of the input so a finding reproduces
    /// exactly, rebuilt between replayed neighbours; then every one.
    ///
    /// A stream that holds a shape the readers themselves cannot read back
    /// is skipped, one named gap at a time (see [`runs_into_a_reader_gap`]).
    pub fn roundtrip_differential(input: &str) {
        let Ok(expected) = decode_with(input, Schema::Yaml12, false, false) else {
            return;
        };
        let Ok(mut stream) = compose_stream_with(input, ParseOptions::default()) else {
            return;
        };
        if runs_into_a_reader_gap(&stream) {
            return;
        }

        check_reemit_roundtrip(input, &stream, &expected, "unmodified");

        let mut scalars = 0;
        for root in &mut stream.roots {
            for_each_scalar(root, &mut |_| scalars += 1);
        }
        if scalars == 0 {
            return;
        }

        let pick = (fnv1a(input.as_bytes()) % scalars as u64) as usize;
        let mut index = 0;
        for root in &mut stream.roots {
            for_each_scalar(root, &mut |scalar| {
                if index == pick {
                    scalar.mark_modified();
                }
                index += 1;
            });
        }
        check_reemit_roundtrip(input, &stream, &expected, "one scalar modified");

        for root in &mut stream.roots {
            for_each_scalar(root, &mut YamlNode::mark_modified);
        }
        check_reemit_roundtrip(input, &stream, &expected, "every scalar modified");
    }

    /// Emit `stream` through the round-trip emitter, reload the output with the
    /// fast decoder, and assert it holds `expected`, document by document.
    /// `stage` names the edit in the panic message.
    fn check_reemit_roundtrip(
        input: &str,
        stream: &ComposedStream,
        expected: &[crate::decode::Value<'_>],
        stage: &str,
    ) {
        let emitted = emit_stream_with(stream, NullStyle::default());
        let text = std::str::from_utf8(&emitted).unwrap_or_else(|e| {
            panic!("round-trip re-emit wrote invalid UTF-8\n  stage:    {stage}\n  input:    {input:?}\n  error:    {e}")
        });
        let reloaded = decode_with(text, Schema::Yaml12, false, false).unwrap_or_else(|e| {
            panic!("round-trip re-emit wrote YAML that loads rejects\n  stage:    {stage}\n  input:    {input:?}\n  emitted:  {text:?}\n  error:    {e:?}")
        });
        assert_eq!(
            reloaded.len(),
            expected.len(),
            "round-trip re-emit changed the number of documents\n  stage:    {stage}\n  input:    {input:?}\n  emitted:  {text:?}"
        );
        for (document, (original, reread)) in expected.iter().zip(&reloaded).enumerate() {
            assert!(
                values_equiv(original, reread),
                "round-trip re-emit changed the data of document {document}\n  stage:    {stage}\n  input:    {input:?}\n  emitted:  {text:?}\n  original: {original:?}\n  reloaded: {reread:?}"
            );
        }
    }

    /// Whether `stream` holds a shape that a known gap in the readers (not
    /// the emitter) keeps from reading back as it was loaded. Each predicate
    /// here names one such gap and the input that shows it; a stream it
    /// matches is skipped, so the target keeps checking everything else.
    pub(crate) fn runs_into_a_reader_gap(stream: &ComposedStream) -> bool {
        stream
            .roots
            .iter()
            .any(|root| any_node(root, &drops_empty_item_before_empty_key))
    }

    /// Whether `node` is a block mapping holding a block sequence whose last
    /// item is a bare `-`, right before an empty key:
    ///
    /// ```yaml
    /// k:
    /// - a
    /// -
    /// : v
    /// ```
    ///
    /// Open problem in the reader, not the emitter: it drops that last item.
    /// The scanner gives an empty key no `Key` token, so the `Value` token
    /// right after the item's `SequenceEntry` ends the sequence, in the fast
    /// decoder and the composer alike. Written out again, a tree holding this
    /// shape loses the item on reload. When the dropped item was the only one,
    /// the tree holds an empty block sequence, which is written as `[]` and
    /// then reads back differently from the source (a tag kept or lost, a
    /// merge key merging nothing). Skipped until both readers keep the item.
    ///
    /// The readers only drop the item when the sequence sits at its key's
    /// own column. Where the emitter puts it depends on the indent it writes
    /// the key at, which is not the key's source column once a parent is
    /// re-indented, so this matches the shape at any column instead of
    /// following the emitter's layout.
    pub(crate) fn drops_empty_item_before_empty_key(node: &YamlNode) -> bool {
        let YamlNodeKind::Mapping(pairs) = &node.kind else {
            return false;
        };

        pairs.windows(2).any(|window| {
            let [(_, value), (next_key, _)] = window else {
                return false;
            };
            let YamlNodeKind::Sequence(items) = &value.kind else {
                return false;
            };

            // A block sequence is never empty in its source; an empty one is
            // what is left once its only item was dropped.
            let dropped = items.last().is_none_or(is_bare_null);
            value.style == NodeStyle::Block && dropped && is_bare_null(next_key)
        })
    }

    /// Whether `node` is a null written as nothing at all: no token, no tag,
    /// no anchor.
    fn is_bare_null(node: &YamlNode) -> bool {
        matches!(node.kind, YamlNodeKind::Null)
            && !node.synthetic
            && node.tag.is_none()
            && node.anchor.is_none()
    }

    /// Whether `predicate` holds for `node` or any node below it, keys
    /// included.
    pub(crate) fn any_node(node: &YamlNode, predicate: &impl Fn(&YamlNode) -> bool) -> bool {
        crate::stack::guard(|| {
            predicate(node)
                || match &node.kind {
                    YamlNodeKind::Mapping(pairs) => pairs
                        .iter()
                        .any(|(key, value)| any_node(key, predicate) || any_node(value, predicate)),
                    YamlNodeKind::Sequence(items) => {
                        items.iter().any(|item| any_node(item, predicate))
                    }
                    YamlNodeKind::Scalar(..) | YamlNodeKind::Alias(_) | YamlNodeKind::Null => false,
                }
        })
    }

    /// Call `visit` on every scalar under `node`, the node itself included, in
    /// document order. Keys are visited too, down into a collection used as
    /// a key: a block scalar key replays its source like any value does.
    fn for_each_scalar(node: &mut YamlNode, visit: &mut impl FnMut(&mut YamlNode)) {
        // The tree is as deep as the input nests, so grow the stack on demand
        // as the emitter's own walks do. See [`crate::stack`].
        crate::stack::guard(|| {
            if matches!(node.kind, YamlNodeKind::Scalar(..)) {
                visit(node);
                return;
            }

            match &mut node.kind {
                YamlNodeKind::Mapping(pairs) => {
                    for (key, value) in pairs {
                        for_each_scalar(key, visit);
                        for_each_scalar(value, visit);
                    }
                }
                YamlNodeKind::Sequence(items) => {
                    for item in items {
                        for_each_scalar(item, visit);
                    }
                }
                YamlNodeKind::Scalar(..) | YamlNodeKind::Alias(_) | YamlNodeKind::Null => {}
            }
        });
    }

    /// The 64-bit FNV-1a hash of `bytes`: a stable, dependency-free way to turn
    /// an input into the same choice on every run.
    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
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
            if !tags_fast_emittable(document) {
                continue;
            }
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
            if !tags_fast_emittable(document) {
                continue;
            }
            for options in &configs {
                check_emit_roundtrip(input, document, options);
            }
        }
    }

    /// The public fast-dump API only accepts tags in directly emittable form:
    /// local/secondary tags (`!foo`, `!!str`) or verbatim tags (`!<uri>`). The
    /// decoder, however, canonicalizes verbatim tags and `%TAG` expansions to
    /// their bare URI forms (`tag:...`), and fast dumps never invents `%TAG`
    /// directives or shorthand handles on output. Skip such values here so the
    /// differential targets fuzz only inputs the fast emitter actually promises to
    /// round-trip.
    ///
    /// The rule is [`crate::encode::emittable_tag`], the one the public API
    /// enforces: checking only the leading `!` here let a `%TAG`-expanded named
    /// handle (`!h!suffix`) through, and the harness then asserted on output that
    /// `dumps` itself refuses to produce.
    fn tags_fast_emittable(value: &crate::decode::Value<'_>) -> bool {
        use crate::decode::Value::{Mapping, Sequence, Tagged};

        match value {
            Tagged(tag, inner) => {
                crate::encode::emittable_tag(tag).is_ok() && tags_fast_emittable(inner)
            }
            Sequence(items) => items.iter().all(tags_fast_emittable),
            Mapping(pairs) => pairs
                .iter()
                .all(|(key, value)| tags_fast_emittable(key) && tags_fast_emittable(value)),
            _ => true,
        }
    }

    /// Resolve `!include`, `!include_dir_*`, `!secret`, and `!env_var` over a
    /// caller-provided directory tree, driving the include engine the `parse`,
    /// `decode`, and `roundtrip` targets never reach: tag-argument parsing, path
    /// confinement, cycle detection, the diamond-lattice expansion budget, the
    /// recursive directory walk, and `secrets.yaml` lookup. All three tag families
    /// are enabled at once so their interaction is exercised too.
    ///
    /// The fuzz target (`fuzz/fuzz_targets/include.rs`) materializes the tree in a
    /// temp directory and passes its canonical path here; this must be the
    /// canonical (symlink-resolved) base so the confinement assertion below is
    /// exact. The contract is the usual never-panic/never-hang/never-UB one *plus*
    /// a hard security invariant: no file the resolver reads may escape the base
    /// directory. A traversal (`!include ../../etc/passwd`) or a symlink swap must
    /// surface as a confinement error, never as a path outside the tree.
    pub fn include(canonical_base: &std::path::Path, root: &str) {
        use crate::include::{IncludeResolver, ResolveTags};

        let tags = ResolveTags {
            includes: true,
            secrets: true,
            env_var: true,
        };
        let mut resolver = IncludeResolver::new(canonical_base, tags).with_dir_recursive(true);

        // Resolution may fail (a missing file, a cycle, a traversal attempt, a
        // blown expansion budget); those are clean errors, not crashes. The
        // property is that it returns rather than panics or hangs.
        let _ = resolver.load_str(root, None);

        // Security invariant: every file the resolver registered (read, or the
        // in-memory root placeholder) must live under the canonical base. A path
        // outside it means confinement was bypassed, the one include bug that is
        // silent rather than a crash.
        let (files, _sources) = resolver.into_parts();
        for path in &files {
            assert!(
                path.starts_with(canonical_base),
                "include escaped the base directory\n  base: {canonical_base:?}\n  path: {path:?}"
            );
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

#[cfg(test)]
mod tests {
    #[test]
    fn differential_skips_canonicalized_tag_uris() {
        // `%TAG` expansion reaches the decoder as a bare URI, but the fast dump
        // API only accepts already-emittable tags starting with `!`.
        let input = "%TAG !e! tag:example.com,2020:\n---\nk: !e!foo bar\n";
        crate::fuzz::differential_options(input);
    }

    #[test]
    fn roundtrip_differential_keeps_multiline_flow_scalars() {
        // A blank line inside a plain or quoted scalar folds to a `\n` in its
        // value. Written raw, that break would put the rest of the value at
        // column 0, where it reads as a document marker or a new key.
        for input in [
            "a: 1\nb: text\n\n  ...\nc: 3\n",
            "a: 1\nb: text\n\n  more\n",
            "r:\n  - x\n\n    y\n",
            "f: [x\n\n  y, z]\n",
            "b: 'x\n\n  y'\n",
            "r:\n  - 'a\n\n    b'\n",
            "a:\n b: x\n\n  y\n",
        ] {
            crate::fuzz::roundtrip_differential(input);
        }
    }

    #[test]
    fn roundtrip_differential_keeps_text_off_column_0() {
        // The emitter writes a root scalar and a root mapping's keys at column
        // 0, where a leading `%` opens a directive and `---` or `...` before
        // white space is a document marker. The source had them indented.
        for input in [
            " %",
            "\t%\t\tx",
            " %z\t\n :-\n  \n\tz7#",
            "\t...\tt",
            " ---\na:- 0X6666 '6663",
            "--- \t%\"r\n-\t ",
            "a\n--- ... >\n",
            " %x: 1\n %y: 2\n",
            " ... x: 1\n",
        ] {
            crate::fuzz::roundtrip_differential(input);
        }
    }

    #[test]
    fn roundtrip_differential_keeps_streams_without_a_root() {
        // Every document of these streams is empty, so no root carries them.
        for input in [
            "---",
            "---\n---\n",
            "%YAML 1.2\n\t\n---\n",
            "---\n# only a comment\n",
            "#\r\r---\r#\r",
            "\u{feff}--- \n",
        ] {
            crate::fuzz::roundtrip_differential(input);
        }
    }

    #[test]
    fn roundtrip_differential_keeps_lone_carriage_returns() {
        // A lone `\r` breaks a line, so the empty lines a `|+` scalar keeps
        // are its own and not blank lines before the next key. And no blank
        // line may follow a `|+` scalar where an entry starts below its `-`.
        for input in [": |+\r\r:", "a: |+\r\r\rb: 1\n", "- |+\n x\n- !t\n\n  y\n"] {
            crate::fuzz::roundtrip_differential(input);
        }
    }

    #[test]
    fn roundtrip_differential_keeps_the_end_of_an_unindented_root_block_scalar() {
        // A root block scalar's lines may start at column 0. A comment after
        // its `...` would otherwise be read as one more of them.
        for input in [
            "--- >\nx\n...\n# c",
            "--- |\n...\n#",
            "a\n--- |\nb\n...\n# c\n",
        ] {
            crate::fuzz::roundtrip_differential(input);
        }
    }

    #[test]
    fn roundtrip_differential_skips_an_empty_item_before_an_empty_key() {
        // The readers drop the bare `-` before `: v`, so a tree that still
        // holds one (or an empty block sequence left by it) is skipped.
        for (input, skipped) in [
            ("k:\n- a\n-\n-\n: v\n", true),
            (": !\n-\n:", true),
            ("<<: !t\n-\n: v\n", true),
            ("k:\n- a\n-\nj: v\n", false),
            ("k:\n  - a\n  -\n: v\n", true),
            // Indented in its source, flush with its key once `outer` is
            // re-indented.
            ("outer:\n k:\n  - a\n  -\n : v\n", true),
            ("k:\n- a\n- !t\n: v\n", false),
        ] {
            let stream =
                crate::roundtrip::composer::compose_stream_with(input, Default::default()).unwrap();
            let matched = stream.roots.iter().any(|root| {
                crate::fuzz::any_node(root, &crate::fuzz::drops_empty_item_before_empty_key)
            });
            assert_eq!(matched, skipped, "{input:?}");
            crate::fuzz::roundtrip_differential(input);
        }
    }

    #[test]
    fn roundtrip_differential_modifies_scalars_in_keys() {
        // A document whose only scalars are keys still reaches the modified
        // stages: a block scalar key, and scalars inside a collection key.
        for input in ["? |\n  x\n:\n", "? [a, 'b c']\n: \n", "? {a: \"x\"}\n:\n"] {
            crate::fuzz::roundtrip_differential(input);
        }
    }

    #[test]
    fn roundtrip_differential_holds_over_the_suite_and_the_corpus() {
        // Every YAML test suite input (when the submodule is checked out) and
        // every file of the curated corpus.
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
        let mut inputs = Vec::new();
        collect_yaml_inputs(&data.join("yaml_test_suite/cases"), &mut inputs);
        collect_yaml_inputs(&data.join("corpus"), &mut inputs);

        for path in inputs {
            if let Ok(input) = std::fs::read_to_string(&path) {
                crate::fuzz::roundtrip_differential(&input);
            }
        }
    }

    /// Every `*.yaml` file under `dir`: for a test suite case, its input and
    /// the expected outputs, which are YAML too. The suite's `name` and `tags`
    /// directories hold the same cases again, so they are left out.
    fn collect_yaml_inputs(dir: &std::path::Path, inputs: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        for path in entries.flatten().map(|entry| entry.path()) {
            let name = path.file_name().unwrap_or_default();
            if path.is_dir() {
                if name != "name" && name != "tags" {
                    collect_yaml_inputs(&path, inputs);
                }
            } else if path.extension().is_some_and(|ext| ext == "yaml") {
                inputs.push(path);
            }
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
    m.add("OPT_TIMESTAMPS", ffi::OPT_TIMESTAMPS)?;

    // The exception hierarchy is defined in pure Python (yamlrocks.exceptions) and
    // raised from Rust via pyo3::import_exception!; it is not registered here.
    m.add_class::<roundtrip::document::YAMLRocksDocument>()?;
    m.add_class::<roundtrip::tree::YAMLRocksTreeNode>()?;
    m.add_class::<roundtrip::document::YAMLRocksDocumentView>()?;
    m.add_class::<roundtrip::document::YAMLRocksNode>()?;
    m.add_class::<ffi::YAMLRocksTag>()?;
    m.add_class::<ffi::YAMLRocksAnnotatedDict>()?;
    m.add_class::<ffi::YAMLRocksAnnotatedList>()?;
    m.add_class::<ffi::YAMLRocksScalar>()?;
    m.add_class::<ffi::YAMLRocksSequence>()?;
    m.add_class::<ffi::YAMLRocksMapping>()?;

    Ok(())
}
