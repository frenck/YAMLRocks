//! Detection of a document's `%YAML <major>.<minor>` version directive.
//!
//! A cheap, allocation-free textual scan of the stream prefix (an optional byte
//! order mark, blank lines, comments, and `%`-directives) for the document's
//! own version declaration. It is pure, parses no body, and does no I/O, in the
//! same spirit as [`crate::schema::schema_ref`].
//!
//! When present, the directive is authoritative for which schema resolves the
//! document's scalars: a `%YAML 1.1` document reads under the 1.1 schema and a
//! `%YAML 1.2` (or any other `1.x`) document under the 1.2 core schema,
//! regardless of the caller's `OPT_YAML_1_1`/`OPT_UPGRADE_1_1` choice. This is
//! what lets an upgraded, stamped file stop being re-interpreted as 1.1.

/// The `(major, minor)` version from the leading `%YAML` directive of the first
/// document, or `None` when the stream declares none.
pub fn leading_yaml_version(input: &str) -> Option<(u32, u32)> {
    // A leading byte order mark is not part of any line's content.
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    for raw in input.lines() {
        let line = raw.trim_start();
        if line.is_empty() {
            // A blank line in the stream prefix; a directive may still follow.
            continue;
        }
        if let Some(rest) = line.strip_prefix('%') {
            // A directive line. Only `%YAML` carries a version; other
            // directives (`%TAG`) are skipped.
            if let Some(version) = directive_version(rest) {
                return version;
            }
            continue;
        }
        if line.starts_with('#') {
            // A comment in the stream prefix; the directive may still follow.
            continue;
        }
        // The first content line, or a `---`/`...` marker, ends the prefix: a
        // `%YAML` directive must appear before it, so there is none.
        break;
    }
    None
}

/// The version declared by one of a document's directives, given its text
/// without the leading `%` (`YAML 1.1`, as the round-trip AST keeps it), or
/// `None` when the directives declare none. Reads them the way
/// [`leading_yaml_version`] reads the stream prefix, so a document of a stream
/// is judged by the same rule as a document loaded on its own.
pub fn declared_yaml_version<'a>(
    directives: impl IntoIterator<Item = &'a str>,
) -> Option<(u32, u32)> {
    directives.into_iter().find_map(directive_version)?
}

/// For a `%YAML` directive (text after the `%`), the version it declares, or
/// `Some(None)` when that version does not parse; `None` for any other
/// directive. An in-line comment (` #...`) is not part of the directive.
fn directive_version(directive: &str) -> Option<Option<(u32, u32)>> {
    let mut parts = directive
        .split_whitespace()
        .take_while(|p| !p.starts_with('#'));
    (parts.next() == Some("YAML")).then(|| parts.next().and_then(parse_version))
}

/// Whether a declared version selects the YAML 1.1 schema. Only `1.1` does; any
/// other `1.x` (and by extension the absence of a directive, handled by the
/// caller) uses the 1.2 core schema.
pub fn selects_yaml_11(version: (u32, u32)) -> bool {
    version == (1, 1)
}

fn parse_version(text: &str) -> Option<(u32, u32)> {
    let (major, minor) = text.split_once('.')?;
    Some((parse_component(major)?, parse_component(minor)?))
}

fn parse_component(component: &str) -> Option<u32> {
    if component.is_empty() || !component.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    component.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{declared_yaml_version, leading_yaml_version};

    #[test]
    fn detects_declared_versions() {
        assert_eq!(leading_yaml_version("%YAML 1.1\n---\nx: 1\n"), Some((1, 1)));
        assert_eq!(leading_yaml_version("%YAML 1.2\n---\nx: 1\n"), Some((1, 2)));
    }

    #[test]
    fn none_when_absent() {
        assert_eq!(leading_yaml_version("x: 1\n"), None);
        assert_eq!(leading_yaml_version("---\nx: 1\n"), None);
    }

    #[test]
    fn skips_prefix_noise() {
        // A BOM, comments, blank lines, and a %TAG directive precede %YAML.
        let input = "\u{feff}# header\n\n%TAG !e! tag:example,2000:\n%YAML 1.1\n---\nx: 1\n";
        assert_eq!(leading_yaml_version(input), Some((1, 1)));
    }

    #[test]
    fn ignores_a_directive_in_the_body() {
        // Once content begins, a later `%YAML` is not a prefix directive.
        assert_eq!(leading_yaml_version("x: 1\n%YAML 1.1\n"), None);
    }

    #[test]
    fn rejects_malformed_version() {
        assert_eq!(leading_yaml_version("%YAML 1\n---\nx: 1\n"), None);
        assert_eq!(leading_yaml_version("%YAML x.y\n---\nx: 1\n"), None);
    }

    #[test]
    fn reads_the_version_a_document_declares() {
        assert_eq!(declared_yaml_version(["YAML 1.1"]), Some((1, 1)));
        assert_eq!(
            declared_yaml_version(["TAG !e! tag:example.com,2000:", "YAML 1.2 # note"]),
            Some((1, 2))
        );
        assert_eq!(
            declared_yaml_version(["TAG !e! tag:example.com,2000:"]),
            None
        );
        assert_eq!(declared_yaml_version(Vec::<&str>::new()), None);
        assert_eq!(declared_yaml_version(["YAML x.y"]), None);
    }
}
