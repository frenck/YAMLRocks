//! Upgrade a round-trip AST from YAML 1.1 to canonical YAML 1.2.
//!
//! Scalars whose type differs between the two schemas (the classic cases being
//! `yes`/`no`/`on`/`off` booleans and `0777`-style octals) are rewritten to
//! their canonical 1.2 spelling and marked modified, so re-emitting the document
//! produces valid 1.2 while preserving comments, layout, and everything that did
//! not need changing.

use crate::emit_util::canonical_float;
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
    use super::upgrade_to_yaml_1_2;
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
}
