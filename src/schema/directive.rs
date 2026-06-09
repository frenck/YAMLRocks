//! Detection of the in-file JSON Schema directive used by the
//! `yaml-language-server` VS Code extension and compatible tooling.
//!
//! The directive is a YAML comment, conventionally the first line of a
//! document, of the form:
//!
//! ```text
//! # yaml-language-server: $schema=https://example.com/schema.json
//! ```
//!
//! It associates a JSON Schema with the document. This module performs a
//! cheap, allocation-light textual scan of the leading comment block to
//! extract the declared `$schema=` reference. It performs no parsing of the
//! document body and no I/O of any kind; resolving the reference to an actual
//! schema (which may involve the network) is left entirely to the caller.

/// Scan the leading comment block of `input` for a `yaml-language-server`
/// schema directive and return the declared reference (the text after
/// `$schema=`, trimmed), or `None` if no directive is present.
///
/// Matching rules, kept deliberately strict so an unrelated comment is never
/// mistaken for a directive:
///
/// - Leading whitespace on the line is allowed.
/// - The line must be a comment: it must start with `#`.
/// - After the `#` (and optional surrounding whitespace) the key must be
///   exactly `yaml-language-server:` followed by `$schema=`.
/// - The value is everything after `$schema=` with surrounding whitespace
///   trimmed; an empty value yields `None`.
///
/// The scan walks the *leading comment block*: blank lines and comment lines
/// at the top of the document. It stops at the first line that is neither
/// blank nor a comment (the document content), because the directive is a
/// header convention and must not be picked up from a comment buried in the
/// body.
pub fn schema_ref(input: &str) -> Option<String> {
    for raw_line in input.lines() {
        let line = raw_line.trim_start();

        // Blank lines are part of the leading block; keep scanning.
        if line.is_empty() {
            continue;
        }

        // The first non-blank, non-comment line is document content. The
        // directive is a header convention, so stop before reading the body.
        let Some(comment) = line.strip_prefix('#') else {
            break;
        };

        if let Some(reference) = parse_directive(comment) {
            return Some(reference);
        }
    }
    None
}

/// Parse the body of a single comment line (the text after the leading `#`)
/// and return the schema reference if it is a well-formed directive.
fn parse_directive(comment: &str) -> Option<String> {
    // Allow `# yaml-language-server: ...` as well as `#yaml-language-server:`.
    let rest = comment.trim_start();
    let rest = rest.strip_prefix("yaml-language-server:")?;
    // The key may be followed by whitespace before `$schema=`.
    let rest = rest.trim_start();
    let value = rest.strip_prefix("$schema=")?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::schema_ref;

    #[test]
    fn detects_first_line_directive() {
        let input = "# yaml-language-server: $schema=https://example.com/s.json\nkey: value\n";
        assert_eq!(
            schema_ref(input).as_deref(),
            Some("https://example.com/s.json")
        );
    }

    #[test]
    fn allows_leading_whitespace_and_blank_lines() {
        let input = "\n\n   #   yaml-language-server: $schema=./local.json\nkey: value\n";
        assert_eq!(schema_ref(input).as_deref(), Some("./local.json"));
    }

    #[test]
    fn tolerates_missing_space_after_hash() {
        let input = "#yaml-language-server: $schema=urn:x\n";
        assert_eq!(schema_ref(input).as_deref(), Some("urn:x"));
    }

    #[test]
    fn trims_trailing_whitespace_from_value() {
        let input = "# yaml-language-server: $schema=urn:x   \n";
        assert_eq!(schema_ref(input).as_deref(), Some("urn:x"));
    }

    #[test]
    fn ignores_plain_comments() {
        let input = "# just a normal comment\nkey: value\n";
        assert_eq!(schema_ref(input), None);
    }

    #[test]
    fn requires_exact_key() {
        let input = "# yaml-language-server-extra: $schema=urn:x\n";
        assert_eq!(schema_ref(input), None);
    }

    #[test]
    fn requires_schema_prefix() {
        let input = "# yaml-language-server: schema=urn:x\n";
        assert_eq!(schema_ref(input), None);
    }

    #[test]
    fn empty_value_is_none() {
        let input = "# yaml-language-server: $schema=\n";
        assert_eq!(schema_ref(input), None);
    }

    #[test]
    fn stops_at_document_body() {
        // A directive that only appears after content must not be detected.
        let input = "key: value\n# yaml-language-server: $schema=urn:x\n";
        assert_eq!(schema_ref(input), None);
    }

    #[test]
    fn finds_directive_below_other_header_comments() {
        let input = concat!(
            "# Copyright 2026\n",
            "# yaml-language-server: $schema=urn:x\n",
            "key: value\n",
        );
        assert_eq!(schema_ref(input).as_deref(), Some("urn:x"));
    }
}
