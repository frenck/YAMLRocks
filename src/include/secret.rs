//! Resolution of the `!secret` tag (`OPT_SECRETS`).
//!
//! `!secret NAME` reads a value from a `secrets.yaml` file, searching from the
//! requesting file's directory up to (and including) the configured base
//! directory. This is a configuration convention shared by tools such as Home
//! Assistant and ESPHome, not specific to any one of them; the exact resolution
//! behavior matches home-assistant-libs/annotatedyaml, the de facto reference.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::roundtrip::ast::{IncludeSource, YamlNode, YamlNodeKind};
use crate::roundtrip::composer::compose_with_file_id;

use super::{scalar_text, IncludeError, IncludeErrorKind, IncludeResolver};

impl IncludeResolver {
    /// Resolve `!secret NAME` by searching `secrets.yaml` from the requesting
    /// file's directory up to (and including) the base directory.
    pub(super) fn resolve_secret(
        &mut self,
        node: &YamlNode,
        stack: &[(PathBuf, u32)],
    ) -> Result<YamlNode, IncludeError> {
        let name = scalar_text(node).unwrap_or_default();
        let requester = self.resolve_path_from_span(node.span);

        // Climb from the requesting file's directory up to the base directory, in
        // canonical space. The requester path and `base_dir` can live in different
        // path spaces (a relative or symlinked `base_dir`, a relative root path,
        // against canonicalized include paths), so the boundary is compared
        // canonical-against-canonical via `canonical_base`, never the raw
        // `base_dir`. Canonicalize the requesting file itself to resolve a
        // relative path to an absolute directory; an in-memory root (no file on
        // disk) starts the climb at the base directory.
        let start = requester
            .canonicalize()
            .ok()
            .and_then(|file| file.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| self.canonical_base.clone());
        let mut dir = Some(start);
        while let Some(current) = dir {
            // Stop once we climb above the config directory.
            if !current.starts_with(&self.canonical_base) {
                break;
            }
            let secrets_path = current.join("secrets.yaml");
            self.ensure_secrets_loaded(&secrets_path)?;
            if let Some(value) = self
                .secrets_cache
                .get(&secrets_path)
                .and_then(|m| m.get(&name))
            {
                let mut resolved = value.clone();
                resolved.span = node.span;
                resolved.source = Some(IncludeSource {
                    tag: "!secret".to_owned(),
                    target: name,
                    target_file_id: None,
                });
                return Ok(resolved);
            }
            if current == self.canonical_base {
                break;
            }
            dir = current.parent().map(Path::to_path_buf);
        }

        // The secret was not found in any `secrets.yaml`. By default this is a
        // hard error (never boot with a hole where a secret belongs). When the
        // caller opted in, collect the miss and resolve to null instead, so every
        // undefined secret is reported in one pass rather than one-per-rerun. Note
        // this downgrades only "name not defined": structural `secrets.yaml`
        // faults already returned via `?` above and still raise.
        if self.collect_missing_secrets {
            self.missing_secrets.push(super::MissingReference {
                name,
                file: requester,
                line: node.span.line,
            });
            return Ok(YamlNode::new(YamlNodeKind::Null, node.span));
        }

        Err(IncludeError {
            kind: IncludeErrorKind::SecretNotFound,
            message: format!("secret '{name}' is not defined in any secrets.yaml"),
            path: requester,
            include_stack: stack.to_vec(),
            span: Some(node.span),
        })
    }

    /// Load and cache a `secrets.yaml` file as a name → value map. A missing
    /// file caches an empty map (so the search can continue upward).
    fn ensure_secrets_loaded(&mut self, path: &Path) -> Result<(), IncludeError> {
        if self.secrets_cache.contains_key(path) {
            return Ok(());
        }
        let mut map = HashMap::new();
        // Only read a secrets.yaml that canonicalizes inside the base tree. A
        // symlink escaping the base (or a missing file) is treated as absent, so
        // the upward search continues without ever reading outside the config.
        // Read the canonical path, not the original: validating one inode and
        // then opening another (the symlink) would reintroduce a TOCTOU window.
        let canonical = path
            .canonicalize()
            .ok()
            .filter(|c| c.starts_with(&self.canonical_base));
        if let Some(canonical) = canonical {
            if let Ok(content) = std::fs::read_to_string(&canonical) {
                let file_id = self.register_file(path.to_path_buf(), Some(content.clone()));
                let nodes = compose_with_file_id(&content, file_id).map_err(|e| IncludeError {
                    kind: IncludeErrorKind::Invalid,
                    message: format!("invalid secrets.yaml: {}", e.message),
                    path: path.to_path_buf(),
                    include_stack: Vec::new(),
                    span: None,
                })?;
                match nodes.first().map(|n| &n.kind) {
                    // An empty secrets.yaml (no document) is an empty secrets set.
                    None | Some(YamlNodeKind::Null) => {}
                    Some(YamlNodeKind::Mapping(pairs)) => {
                        for (key, val) in pairs {
                            // `secrets.yaml` is plain data: a `!secret` nested
                            // inside it is rejected at load time, not honored, so
                            // secrets cannot reference other secrets.
                            if contains_secret_tag(val) {
                                return Err(IncludeError {
                                    kind: IncludeErrorKind::Invalid,
                                    message: "secrets not supported in a secrets.yaml file".into(),
                                    path: path.to_path_buf(),
                                    include_stack: Vec::new(),
                                    span: None,
                                });
                            }
                            if let Some(key_name) = scalar_text(key) {
                                // `logger` is reserved (it controls the secret
                                // logger's level), so it is never a usable secret.
                                // Its only valid value is `debug`; anything else
                                // is a non-fatal misconfiguration worth surfacing.
                                if key_name == "logger" {
                                    let value = scalar_text(val).unwrap_or_default();
                                    if !value.eq_ignore_ascii_case("debug") {
                                        self.warn(format!(
                                            "secrets.yaml: 'logger: debug' expected, but \
                                             'logger: {value}' found"
                                        ));
                                    }
                                } else {
                                    map.insert(key_name, val.clone());
                                }
                            }
                        }
                    }
                    // A non-mapping secrets.yaml (e.g. a list) is a configuration
                    // error, not an empty secrets set.
                    Some(_) => {
                        return Err(IncludeError {
                            kind: IncludeErrorKind::Invalid,
                            message: "secrets.yaml does not contain a dictionary".into(),
                            path: path.to_path_buf(),
                            include_stack: Vec::new(),
                            span: None,
                        });
                    }
                }
            }
        }
        self.secrets_cache.insert(path.to_path_buf(), map);
        Ok(())
    }
}

/// Whether `node` (or anything nested within it) carries a `!secret` tag. Used
/// to reject a `!secret` appearing inside a `secrets.yaml` file, where secrets
/// are plain data and must not reference one another.
fn contains_secret_tag(node: &YamlNode) -> bool {
    if node.tag.as_deref() == Some("!secret") {
        return true;
    }
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => pairs
            .iter()
            .any(|(k, v)| contains_secret_tag(k) || contains_secret_tag(v)),
        YamlNodeKind::Sequence(items) => items.iter().any(contains_secret_tag),
        _ => false,
    }
}
