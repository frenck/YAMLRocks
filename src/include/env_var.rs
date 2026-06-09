//! Resolution of the `!env_var` tag (`OPT_ENV_VAR`).
//!
//! `!env_var NAME [default words]` reads an environment variable, falling back
//! to the default text when the variable is unset and erroring on a bare,
//! undefined variable. This is a configuration convention shared by tools such
//! as Home Assistant and ESPHome, not specific to any one of them; the behavior
//! matches home-assistant-libs/annotatedyaml, the de facto reference.

use std::path::PathBuf;

use crate::roundtrip::ast::{IncludeSource, YamlNode, YamlNodeKind};
use crate::scanner::ScalarStyle;

use super::{scalar_text, IncludeError, IncludeErrorKind, IncludeResolver};

impl IncludeResolver {
    /// Resolve `!env_var NAME [default words]`: returns the environment
    /// variable, or the default when more than one word is given, or errors
    /// when a bare variable is undefined.
    pub(super) fn resolve_env_var(
        &mut self,
        node: &YamlNode,
        stack: &[(PathBuf, u32)],
    ) -> Result<YamlNode, IncludeError> {
        let text = scalar_text(node).unwrap_or_default();
        let args: Vec<&str> = text.split_whitespace().collect();
        let value = if args.len() > 1 {
            std::env::var(args[0]).unwrap_or_else(|_| args[1..].join(" "))
        } else if let Some(name) = args.first() {
            match std::env::var(name) {
                Ok(value) => value,
                // A bare, undefined variable is a hard error by default. When the
                // caller opted in, collect the miss and resolve to null instead,
                // so every undefined variable is reported in one pass. A variable
                // given a default never reaches here (it used the default above).
                Err(_) if self.collect_missing_env_vars => {
                    self.missing_env_vars.push(super::MissingReference {
                        name: (*name).to_owned(),
                        file: self.resolve_path_from_span(node.span),
                        line: node.span.line,
                    });
                    return Ok(YamlNode::new(YamlNodeKind::Null, node.span));
                }
                Err(_) => {
                    return Err(IncludeError {
                        kind: IncludeErrorKind::EnvVarUndefined,
                        message: format!("environment variable {name} is not defined"),
                        path: self.resolve_path_from_span(node.span),
                        include_stack: stack.to_vec(),
                        span: Some(node.span),
                    })
                }
            }
        } else {
            String::new()
        };

        // Environment variables are always strings; a quoted style keeps the
        // resolver from coercing e.g. "8080" to an int.
        let mut resolved = YamlNode::new(
            YamlNodeKind::Scalar(value, ScalarStyle::SingleQuoted),
            node.span,
        );
        resolved.source = Some(IncludeSource {
            tag: "!env_var".to_owned(),
            target: text,
            target_file_id: None,
        });
        Ok(resolved)
    }
}
