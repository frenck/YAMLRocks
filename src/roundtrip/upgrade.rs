//! Upgrade a round-trip AST from YAML 1.1 to canonical YAML 1.2.
//!
//! Scalars whose type differs between the two schemas (the classic cases being
//! `yes`/`no`/`on`/`off` booleans and `0777`-style octals) are rewritten to
//! their canonical 1.2 spelling and marked modified, so re-emitting the document
//! produces valid 1.2 while preserving comments, layout, and everything that did
//! not need changing.

use std::collections::HashSet;

use crate::emit_util::canonical_float;
use crate::parser::Parser;
use crate::resolver::{ResolvedValue, Schema};
use crate::scanner::ScalarStyle;

use super::ast::{YamlNode, YamlNodeKind};

/// Rewrite 1.1-only scalar spellings to canonical 1.2 across the document.
/// `schema` is the 1.1 reading schema (spec or PyYAML), so under PyYAML-compat
/// bare `y`/`n` are not booleans and are left untouched.
pub fn upgrade_to_yaml_1_2(nodes: &mut [YamlNode], schema: Schema) {
    for node in nodes {
        upgrade_node(node, schema);
    }
}

/// Mark every file whose layout only the lenient PyYAML-compat reader accepts as
/// modified, so the upgrade rewrites it.
///
/// Under PyYAML compatibility a file can lay out a flow collection at its
/// block's indent, which strict YAML rejects. An upgrade stamps `%YAML 1.2` and
/// promises output the strict reader loads, so such a file needs rewriting just
/// like a 1.1-only scalar: marked modified, it re-emits canonically from the AST
/// and `save()` writes it. A file that is valid YAML anyway keeps its bytes.
///
/// `root_source` is the document's own text. `file_sources` holds each included
/// file's original text by `file_id`; index 0 is the root, which `root_source`
/// already covers.
pub fn mark_lenient_only_files_modified(
    nodes: &mut [YamlNode],
    root_source: &str,
    file_sources: &[Option<String>],
) {
    if !parses_strictly(root_source) {
        for node in nodes.iter_mut() {
            node.mark_modified();
        }
    }

    let lenient_files: HashSet<u32> = file_sources
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, source)| source.as_deref().is_some_and(|text| !parses_strictly(text)))
        .filter_map(|(file_id, _)| u32::try_from(file_id).ok())
        .collect();
    if lenient_files.is_empty() {
        return;
    }

    for node in nodes {
        mark_include_boundaries(node, &lenient_files);
    }
}

fn parses_strictly(source: &str) -> bool {
    Parser::new(source).parse_all().is_ok()
}

/// Mark each include boundary whose target file is in `files` as modified. The
/// boundary owns its file, so this re-emits that file and leaves the rest alone.
fn mark_include_boundaries(node: &mut YamlNode, files: &HashSet<u32>) {
    // Recurses once per nesting level over attacker-controlled depth.
    crate::stack::guard(|| {
        let target = node
            .source
            .as_ref()
            .and_then(|source| source.target_file_id);
        if target.is_some_and(|file_id| files.contains(&file_id)) {
            node.mark_modified();
        }

        match &mut node.kind {
            YamlNodeKind::Mapping(pairs) => {
                for (key, value) in pairs {
                    mark_include_boundaries(key, files);
                    mark_include_boundaries(value, files);
                }
            }
            YamlNodeKind::Sequence(items) => {
                for item in items {
                    mark_include_boundaries(item, files);
                }
            }
            _ => {}
        }
    });
}

fn upgrade_node(node: &mut YamlNode, schema: Schema) {
    match &mut node.kind {
        YamlNodeKind::Scalar(text, style) => {
            // Only plain scalars are schema-ambiguous; quoted/block scalars are
            // always strings in both schemas.
            if *style != ScalarStyle::Plain {
                return;
            }
            let tag = node.tag.as_deref();
            let v11 = schema.resolve(text, *style, tag);
            let v12 = Schema::Yaml12.resolve(text, *style, tag);
            if v11 != v12 {
                if let Some(canonical) = canonical_1_2(&v11) {
                    *text = canonical;
                    node.comments.modified = true;
                }
            }
        }
        YamlNodeKind::Mapping(pairs) => {
            for (key, val) in pairs.iter_mut() {
                upgrade_node(key, schema);
                upgrade_node(val, schema);
            }
        }
        YamlNodeKind::Sequence(items) => {
            for item in items.iter_mut() {
                upgrade_node(item, schema);
            }
        }
        _ => {}
    }
}

/// The canonical YAML 1.2 plain-scalar spelling of a resolved value, or `None`
/// when no rewrite is needed (e.g. the value is already a string).
fn canonical_1_2(value: &ResolvedValue) -> Option<String> {
    match value {
        ResolvedValue::Bool(true) => Some("true".to_owned()),
        ResolvedValue::Bool(false) => Some("false".to_owned()),
        ResolvedValue::Null => Some("null".to_owned()),
        ResolvedValue::Int(i) => Some(i.to_string()),
        ResolvedValue::BigInt(s) => Some(s.clone()),
        ResolvedValue::Float(f) => Some(canonical_float(*f)),
        ResolvedValue::String(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{mark_lenient_only_files_modified, upgrade_to_yaml_1_2};
    use crate::emit_util::canonical_float;
    use crate::roundtrip::ast::{YamlNode, YamlNodeKind};
    use crate::scanner::{ScalarStyle, Span};

    fn span() -> Span {
        Span {
            file_id: 0,
            line: 0,
            column: 0,
            offset: 0,
        }
    }
    fn plain(text: &str) -> YamlNode {
        YamlNode::new(
            YamlNodeKind::Scalar(text.to_owned(), ScalarStyle::Plain),
            span(),
        )
    }
    fn text(node: &YamlNode) -> &str {
        let YamlNodeKind::Scalar(t, _) = &node.kind else {
            panic!("expected a scalar");
        };
        t
    }

    #[test]
    fn rewrites_yes_to_true() {
        let mut nodes = vec![plain("yes")];
        upgrade_to_yaml_1_2(&mut nodes, crate::resolver::Schema::Yaml11);
        assert_eq!(text(&nodes[0]), "true");
        assert!(nodes[0].comments.modified);
    }

    #[test]
    fn rewrites_octal_to_decimal() {
        let mut nodes = vec![plain("0777")];
        upgrade_to_yaml_1_2(&mut nodes, crate::resolver::Schema::Yaml11);
        assert_eq!(text(&nodes[0]), "511");
    }

    #[test]
    fn leaves_plain_strings_untouched() {
        let mut nodes = vec![plain("hello")];
        upgrade_to_yaml_1_2(&mut nodes, crate::resolver::Schema::Yaml11);
        assert_eq!(text(&nodes[0]), "hello");
        assert!(!nodes[0].comments.modified);
    }

    #[test]
    fn ignores_quoted_scalars() {
        let mut nodes = vec![YamlNode::new(
            YamlNodeKind::Scalar("yes".to_owned(), ScalarStyle::SingleQuoted),
            span(),
        )];
        upgrade_to_yaml_1_2(&mut nodes, crate::resolver::Schema::Yaml11);
        assert_eq!(text(&nodes[0]), "yes");
    }

    #[test]
    fn upgrades_nested_values() {
        let mut nodes = vec![YamlNode::new(
            YamlNodeKind::Mapping(vec![(plain("k"), plain("on"))]),
            span(),
        )];
        upgrade_to_yaml_1_2(&mut nodes, crate::resolver::Schema::Yaml11);
        let YamlNodeKind::Mapping(pairs) = &nodes[0].kind else {
            panic!("mapping");
        };
        assert_eq!(text(&pairs[0].1), "true");
    }

    #[test]
    fn canonical_float_spellings() {
        assert_eq!(canonical_float(f64::INFINITY), ".inf");
        assert_eq!(canonical_float(f64::NEG_INFINITY), "-.inf");
        assert_eq!(canonical_float(f64::NAN), ".nan");
        assert_eq!(canonical_float(1.5), "1.5");
        assert_eq!(canonical_float(3.0), "3.0");
    }

    #[test]
    fn only_a_lenient_only_root_is_marked_for_rewrite() {
        let lenient = "command: [\n  'a'\n]\n";
        let mut nodes = vec![plain("x")];
        mark_lenient_only_files_modified(&mut nodes, lenient, &[]);
        assert!(nodes[0].comments.modified);

        let valid = "command: [\n  'a'\n  ]\n";
        let mut nodes = vec![plain("x")];
        mark_lenient_only_files_modified(&mut nodes, valid, &[]);
        assert!(!nodes[0].comments.modified);
    }
}
