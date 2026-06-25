use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::roundtrip::ast::{IncludeSource, YamlNode, YamlNodeKind};
use crate::roundtrip::composer::compose_with_file_id;
use crate::scanner::{ScalarStyle, Span};

// `!env_var` and `!secret` are two distinct, independently gated tag features;
// each lives in its own module with its own resolver. Both extend
// `IncludeResolver` with `pub(super)` methods that the dispatch below calls.
mod env_var;
mod secret;

/// Maximum depth of a chain of (acyclic) `!include`/`!include_dir_*` directives.
/// Cycle detection already rejects self-referential chains; this bounds a long
/// linear chain of distinct files so it cannot exhaust the stack or memory.
const MAX_INCLUDE_DEPTH: usize = 50;

/// Which application tag families the resolver should expand. Each tag crosses
/// a different trust boundary (`!include` reads files, `!secret` reads a secrets
/// file, `!env_var` reads the process environment), so each is opt-in on its
/// own. A tag whose flag is off is left intact and handled downstream as an
/// ordinary custom tag.
#[derive(Clone, Copy, Default)]
pub struct ResolveTags {
    pub includes: bool,
    pub secrets: bool,
    pub env_var: bool,
}

impl ResolveTags {
    /// Whether any tag family is enabled (i.e. the resolver needs to run).
    pub fn any(&self) -> bool {
        self.includes || self.secrets || self.env_var
    }
}

/// Resolves `!include` and `!include_dir_*` tags into their target content.
///
/// Produces an AST where include tags are replaced by the included content,
/// with each node's span stamped with the `file_id` it came from. The
/// `file_id` → path mapping ([`into_parts`](Self::into_parts)) drives
/// write-back to the correct source files.
pub struct IncludeResolver {
    base_dir: PathBuf,
    /// `base_dir` resolved to a canonical, symlink-free path. Every resolved
    /// include/secret path must stay under this boundary.
    canonical_base: PathBuf,
    file_map: Vec<PathBuf>,
    /// Original source text of each registered file, indexed by `file_id`
    /// (parallel to `file_map`). Lets the writable-include path re-emit an
    /// unmodified included file byte-for-byte instead of from the AST.
    file_sources: Vec<Option<String>>,
    /// Cache of loaded `secrets.yaml` files: directory path → (name → value).
    secrets_cache: HashMap<PathBuf, HashMap<String, YamlNode>>,
    tags: ResolveTags,
    /// Whether `!include_dir_*` descends into subdirectories (`os.walk`-style).
    /// Off by default (top level only); enabled via `OPT_INCLUDE_DIR_RECURSIVE`.
    dir_recursive: bool,
    /// Non-fatal diagnostics gathered during resolution (e.g. a non-`debug`
    /// `logger:` value in a `secrets.yaml`), surfaced to the caller to emit
    /// through Python logging.
    warnings: Vec<String>,
    /// When set, an undefined `!secret` is collected in `missing_secrets` and
    /// resolved to null rather than raising, so the caller can gather every miss
    /// in one pass (`OPT_SECRET_NOT_FOUND_WARN` or an `on_missing_secret`
    /// callback). Structural `secrets.yaml` faults still raise regardless.
    collect_missing_secrets: bool,
    /// Undefined `!secret` references gathered when `collect_missing_secrets` is
    /// set, in resolution (document) order, for the caller to report.
    missing_secrets: Vec<MissingReference>,
    /// When set, an undefined `!env_var` with no default is collected in
    /// `missing_env_vars` and resolved to null rather than raising
    /// (`OPT_ENV_VAR_NOT_FOUND_WARN` or an `on_missing_env_var` callback).
    collect_missing_env_vars: bool,
    /// Undefined `!env_var` references gathered when `collect_missing_env_vars`
    /// is set, in resolution order.
    missing_env_vars: Vec<MissingReference>,
}

/// An undefined config-tag reference (`!secret NAME` or a bare `!env_var NAME`),
/// collected (instead of raised) when the caller opted into gathering misses.
/// Carries what a diagnostic needs: the referenced name and the location of the
/// requesting tag. Never carries a resolved value.
pub struct MissingReference {
    pub name: String,
    /// The file containing the reference (a real path, or the in-memory root
    /// placeholder).
    pub file: PathBuf,
    /// 0-based line of the reference (the caller adds 1 for display).
    pub line: u32,
}

impl IncludeResolver {
    pub fn new(base_dir: impl Into<PathBuf>, tags: ResolveTags) -> Self {
        let base_dir = base_dir.into();
        // Resolve the boundary once. A base that does not yet exist falls back
        // to a lexical normalization so confinement still rejects `..` escapes.
        // `dunce::canonicalize` avoids Windows' `\\?\` verbatim prefix, so the
        // base compares equal to lexically-normalized candidates on every OS.
        let canonical_base =
            dunce::canonicalize(&base_dir).unwrap_or_else(|_| lexical_normalize(&base_dir));
        Self {
            base_dir,
            canonical_base,
            file_map: Vec::new(),
            file_sources: Vec::new(),
            secrets_cache: HashMap::new(),
            tags,
            dir_recursive: false,
            warnings: Vec::new(),
            collect_missing_secrets: false,
            missing_secrets: Vec::new(),
            collect_missing_env_vars: false,
            missing_env_vars: Vec::new(),
        }
    }

    /// Take the non-fatal diagnostics gathered during resolution, leaving the
    /// resolver's list empty. Call before consuming the resolver to surface them.
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    /// Record a non-fatal diagnostic for the caller to log.
    pub(super) fn warn(&mut self, message: String) {
        self.warnings.push(message);
    }

    /// Enable recursive directory walking for the `!include_dir_*` tags, so they
    /// descend into subdirectories instead of reading only the top level.
    pub fn with_dir_recursive(mut self, recursive: bool) -> Self {
        self.dir_recursive = recursive;
        self
    }

    /// Collect undefined `!secret` references instead of raising on the first, so
    /// the caller can report every miss in one pass. Resolves each missing secret
    /// to null and continues; structural `secrets.yaml` faults still raise.
    pub fn with_collect_missing_secrets(mut self, collect: bool) -> Self {
        self.collect_missing_secrets = collect;
        self
    }

    /// Take the undefined-secret references gathered during resolution, leaving
    /// the resolver's list empty. Call before consuming the resolver.
    pub fn take_missing_secrets(&mut self) -> Vec<MissingReference> {
        std::mem::take(&mut self.missing_secrets)
    }

    /// Collect undefined `!env_var` references (a bare variable with no default
    /// that is unset) instead of raising on the first. Resolves each to null and
    /// continues.
    pub fn with_collect_missing_env_vars(mut self, collect: bool) -> Self {
        self.collect_missing_env_vars = collect;
        self
    }

    /// Take the undefined-env-var references gathered during resolution.
    pub fn take_missing_env_vars(&mut self) -> Vec<MissingReference> {
        std::mem::take(&mut self.missing_env_vars)
    }

    /// Register a file (with its original source, when known) and return its
    /// `file_id`.
    fn register_file(&mut self, path: PathBuf, source: Option<String>) -> u32 {
        let id = self.file_map.len() as u32;
        self.file_map.push(path);
        self.file_sources.push(source);
        id
    }

    /// Consume the resolver and return both the `file_id` → path map and the
    /// `file_id` → original-source map, for byte-exact include write-back and
    /// for validating each included file's source.
    pub fn into_parts(self) -> (Vec<PathBuf>, Vec<Option<String>>) {
        (self.file_map, self.file_sources)
    }

    /// Resolve includes in an in-memory document. The content is registered as
    /// the root file (`file_id` 0); includes resolve relative to `base_dir`.
    pub fn load_str(
        &mut self,
        content: &str,
        root_path: Option<PathBuf>,
    ) -> Result<Vec<YamlNode>, IncludeError> {
        // Register the root document under its real path when the caller knows
        // it (e.g. `load(path)`), so root nodes report that path and includes in
        // the root resolve relative to its own directory. When there is no path
        // (in-memory `loads(bytes)`), fall back to a placeholder *inside* the
        // base directory so includes still resolve relative to `base_dir`.
        let root = root_path.unwrap_or_else(|| self.base_dir.join("<root>"));
        let file_id = self.register_file(root.clone(), Some(content.to_owned()));

        let nodes = compose_with_file_id(content, file_id).map_err(|e| IncludeError {
            kind: IncludeErrorKind::Parse,
            message: e.message,
            path: root.clone(),
            include_stack: Vec::new(),
            span: Some(e.span),
        })?;

        let mut resolved = Vec::new();
        for node in nodes {
            resolved.push(self.resolve_node(node, &[])?);
        }
        Ok(resolved)
    }

    fn resolve_node(
        &mut self,
        node: YamlNode,
        stack: &[(PathBuf, u32)],
    ) -> Result<YamlNode, IncludeError> {
        // Grow the native stack on demand so resolving tags/includes over a
        // deeply nested document cannot overflow a small thread stack. See
        // [`crate::stack`].
        crate::stack::guard(|| self.resolve_node_inner(node, stack))
    }

    fn resolve_node_inner(
        &mut self,
        node: YamlNode,
        stack: &[(PathBuf, u32)],
    ) -> Result<YamlNode, IncludeError> {
        // Expand the application tags whose flag is enabled. A disabled tag
        // falls through and is left intact (handled later as a custom tag).
        if let Some(ref tag) = node.tag {
            match tag.as_str() {
                "!include" if self.tags.includes => return self.resolve_include(&node, stack),
                "!include_dir_named" if self.tags.includes => {
                    return self.resolve_include_dir_named(&node, stack)
                }
                "!include_dir_list" if self.tags.includes => {
                    return self.resolve_include_dir_list(&node, stack)
                }
                "!include_dir_merge_named" if self.tags.includes => {
                    return self.resolve_include_dir_merge_named(&node, stack)
                }
                "!include_dir_merge_list" if self.tags.includes => {
                    return self.resolve_include_dir_merge_list(&node, stack)
                }
                "!env_var" if self.tags.env_var => return self.resolve_env_var(&node, stack),
                "!secret" if self.tags.secrets => return self.resolve_secret(&node, stack),
                _ => {}
            }
        }

        // Recursively resolve children
        match node.kind {
            YamlNodeKind::Mapping(pairs) => {
                let mut resolved_pairs = Vec::new();
                for (key, val) in pairs {
                    let resolved_val = self.resolve_node(val, stack)?;
                    resolved_pairs.push((key, resolved_val));
                }
                Ok(YamlNode {
                    kind: YamlNodeKind::Mapping(resolved_pairs),
                    ..node
                })
            }
            YamlNodeKind::Sequence(items) => {
                let mut resolved_items = Vec::new();
                for item in items {
                    resolved_items.push(self.resolve_node(item, stack)?);
                }
                Ok(YamlNode {
                    kind: YamlNodeKind::Sequence(resolved_items),
                    ..node
                })
            }
            _ => Ok(node),
        }
    }

    fn resolve_include(
        &mut self,
        node: &YamlNode,
        stack: &[(PathBuf, u32)],
    ) -> Result<YamlNode, IncludeError> {
        let filename = self.scalar_arg(node, "!include", stack)?;

        self.check_include_depth(node.span, stack)?;
        let include_path = self.resolve_path(&filename, node.span, stack)?;

        // Detect include cycles (a.yaml -> b.yaml -> a.yaml) before recursing.
        if stack.iter().any(|(path, _)| path == &include_path) {
            return Err(IncludeError {
                kind: IncludeErrorKind::Circular,
                message: format!("circular include detected: '{filename}'"),
                path: include_path,
                include_stack: stack.to_vec(),
                span: None,
            });
        }

        let (file_id, nodes) = self.read_and_compose(&include_path, &filename, stack)?;

        let mut new_stack = stack.to_vec();
        new_stack.push((include_path, node.span.line));

        let mut resolved = if let Some(first) = nodes.into_iter().next() {
            self.resolve_node(first, &new_stack)?
        } else {
            // An empty included file normalizes to an empty mapping, matching
            // annotatedyaml (a missing value is configuration absence, not null).
            YamlNode::new(YamlNodeKind::Mapping(Vec::new()), node.span)
        };
        // Remember the directive so it can be rewritten into its parent file.
        resolved.source = Some(IncludeSource {
            tag: "!include".to_owned(),
            target: filename,
            target_file_id: Some(file_id),
        });
        Ok(resolved)
    }

    fn resolve_include_dir_named(
        &mut self,
        node: &YamlNode,
        stack: &[(PathBuf, u32)],
    ) -> Result<YamlNode, IncludeError> {
        let dirname = self.scalar_arg(node, "!include_dir_named", stack)?;

        self.check_include_depth(node.span, stack)?;
        let dir_path = self.resolve_path(&dirname, node.span, stack)?;
        let mut pairs = Vec::new();

        if dir_path.is_dir() {
            for (stem, path) in self.collect_dir_files(&dir_path, node.span, stack)? {
                let (file_id, nodes) =
                    self.read_and_compose(&path, &path.to_string_lossy(), stack)?;

                let mut new_stack = stack.to_vec();
                new_stack.push((path.clone(), node.span.line));
                // Resolve nested `!include`/`!secret`/`!env_var` so a directory
                // include behaves like a plain `!include` of each file. An empty
                // file contributes an empty mapping, matching the single
                // `!include` normalization rather than a bare null.
                let val = if let Some(first) = nodes.into_iter().next() {
                    self.resolve_node(first, &new_stack)?
                } else {
                    YamlNode::new(YamlNodeKind::Mapping(Vec::new()), node.span)
                };

                // Annotate the synthetic stem key with the included file's own
                // location (its line 1), not the `!include_dir_named` directive
                // site, so an error in the file is attributed to the file.
                let key_span = Span::new(file_id, 0, 0, 0);
                let key = YamlNode::new(YamlNodeKind::Scalar(stem, ScalarStyle::Plain), key_span);
                pairs.push((key, val));
            }
        }

        let mut result = YamlNode::new(YamlNodeKind::Mapping(pairs), node.span);
        result.source = Some(IncludeSource {
            tag: "!include_dir_named".to_owned(),
            target: dirname,
            target_file_id: None,
        });
        Ok(result)
    }

    fn resolve_include_dir_list(
        &mut self,
        node: &YamlNode,
        stack: &[(PathBuf, u32)],
    ) -> Result<YamlNode, IncludeError> {
        let dirname = self.scalar_arg(node, "!include_dir_list", stack)?;

        self.check_include_depth(node.span, stack)?;
        let dir_path = self.resolve_path(&dirname, node.span, stack)?;
        let mut items = Vec::new();

        if dir_path.is_dir() {
            for (_, path) in self.collect_dir_files(&dir_path, node.span, stack)? {
                let (_, nodes) = self.read_and_compose(&path, &path.to_string_lossy(), stack)?;

                let mut new_stack = stack.to_vec();
                new_stack.push((path.clone(), node.span.line));
                // Resolve nested directives in each listed file, as `!include`
                // does for a single file. An empty file yields no nodes and so
                // contributes nothing to the list (matching annotatedyaml).
                for n in nodes {
                    items.push(self.resolve_node(n, &new_stack)?);
                }
            }
        }

        let mut result = YamlNode::new(YamlNodeKind::Sequence(items), node.span);
        result.source = Some(IncludeSource {
            tag: "!include_dir_list".to_owned(),
            target: dirname,
            target_file_id: None,
        });
        Ok(result)
    }

    /// Enumerate the YAML files a `!include_dir_*` tag should load from `dir`,
    /// returning each as `(stem, confined_path)` where `stem` is the file's base
    /// name without extension (the mapping key for the `*_named` forms).
    ///
    /// Mirrors annotatedyaml's `_find_files`: results are sorted, hidden entries
    /// (names beginning with `.`) are skipped, and `secrets.yaml` is skipped when
    /// the `!secret` feature is active (it is configuration, not content). When
    /// recursion is enabled, subdirectories are walked top-down with each
    /// directory's files and subdirectories visited in sorted order, so a stable
    /// top-level-then-deeper ordering results regardless of filesystem order.
    fn collect_dir_files(
        &self,
        dir: &Path,
        span: Span,
        stack: &[(PathBuf, u32)],
    ) -> Result<Vec<(String, PathBuf)>, IncludeError> {
        let read = |path: &Path| -> Result<Vec<std::fs::DirEntry>, IncludeError> {
            let mut entries: Vec<_> = std::fs::read_dir(path)
                .map_err(|e| IncludeError {
                    kind: IncludeErrorKind::NotFound,
                    message: format!("cannot read directory: {e}"),
                    path: path.to_path_buf(),
                    include_stack: stack.to_vec(),
                    span: None,
                })?
                .filter_map(Result::ok)
                .collect();
            entries.sort_by_key(std::fs::DirEntry::file_name);
            Ok(entries)
        };

        let mut out = Vec::new();
        // An explicit work stack keeps the walk iterative (no recursion limit of
        // its own) and preserves top-down, sorted order: a directory's files are
        // emitted before its subdirectories are descended into.
        let mut dirs = vec![dir.to_path_buf()];
        while let Some(current) = dirs.pop() {
            let entries = read(&current)?;
            let mut subdirs = Vec::new();
            for entry in entries {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                // Skip hidden files and directories (dotfiles), like os.walk
                // filtered by annotatedyaml's `_is_file_valid`.
                if name.starts_with('.') {
                    continue;
                }
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    if self.dir_recursive {
                        // Confine subdirectories too, so a symlinked directory
                        // cannot redirect the walk outside the base tree.
                        subdirs.push(self.confine(
                            entry_path.clone(),
                            &entry_path.to_string_lossy(),
                            span,
                            stack,
                        )?);
                    }
                    continue;
                }
                // Only YAML files participate.
                if !entry_path
                    .extension()
                    .is_some_and(|ext| ext == "yaml" || ext == "yml")
                {
                    continue;
                }
                // `secrets.yaml` is configuration for the `!secret` feature, not
                // content to include; skip it only when that feature is enabled.
                if self.tags.secrets && name == "secrets.yaml" {
                    continue;
                }
                // Confine each entry (following symlinks) before reading, so a
                // symlink inside the directory cannot escape the base tree. Key
                // on the entry's own stem, not the resolved target, so a
                // symlinked entry keeps its name.
                let confined = self.confine(
                    entry_path.clone(),
                    &entry_path.to_string_lossy(),
                    span,
                    stack,
                )?;
                let stem = entry_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                out.push((stem, confined));
            }
            // Push subdirectories in reverse so the `pop()` above visits them in
            // ascending (sorted) order, immediately after the current directory.
            for subdir in subdirs.into_iter().rev() {
                dirs.push(subdir);
            }
        }
        Ok(out)
    }

    fn resolve_include_dir_merge_named(
        &mut self,
        node: &YamlNode,
        stack: &[(PathBuf, u32)],
    ) -> Result<YamlNode, IncludeError> {
        let dirname = scalar_text(node).unwrap_or_default();
        let result = self.resolve_include_dir_named(node, stack)?;
        // Merge all per-file mappings into one.
        let mut merged_node = if let YamlNodeKind::Mapping(pairs) = result.kind {
            let mut merged = Vec::new();
            for (_, val) in pairs {
                if let YamlNodeKind::Mapping(inner_pairs) = val.kind {
                    merged.extend(inner_pairs);
                }
            }
            YamlNode::new(YamlNodeKind::Mapping(merged), node.span)
        } else {
            result
        };
        merged_node.source = Some(IncludeSource {
            tag: "!include_dir_merge_named".to_owned(),
            target: dirname,
            target_file_id: None,
        });
        Ok(merged_node)
    }

    fn resolve_include_dir_merge_list(
        &mut self,
        node: &YamlNode,
        stack: &[(PathBuf, u32)],
    ) -> Result<YamlNode, IncludeError> {
        let dirname = scalar_text(node).unwrap_or_default();
        let result = self.resolve_include_dir_list(node, stack)?;
        // Flatten: if items are themselves sequences, merge them.
        let mut merged_node = if let YamlNodeKind::Sequence(items) = result.kind {
            let mut merged = Vec::new();
            for item in items {
                if let YamlNodeKind::Sequence(inner) = item.kind {
                    merged.extend(inner);
                } else {
                    merged.push(item);
                }
            }
            YamlNode::new(YamlNodeKind::Sequence(merged), node.span)
        } else {
            result
        };
        merged_node.source = Some(IncludeSource {
            tag: "!include_dir_merge_list".to_owned(),
            target: dirname,
            target_file_id: None,
        });
        Ok(merged_node)
    }

    /// Reject an include chain that has grown past [`MAX_INCLUDE_DEPTH`], even if
    /// it is acyclic. Bounds stack/memory use against a long linear chain.
    fn check_include_depth(
        &self,
        span: Span,
        stack: &[(PathBuf, u32)],
    ) -> Result<(), IncludeError> {
        if stack.len() >= MAX_INCLUDE_DEPTH {
            return Err(IncludeError {
                kind: IncludeErrorKind::Depth,
                message: format!("include chain exceeds the maximum depth of {MAX_INCLUDE_DEPTH}"),
                path: self.resolve_path_from_span(span),
                include_stack: stack.to_vec(),
                span: None,
            });
        }
        Ok(())
    }

    /// Join an include argument to the requesting file's directory and confine
    /// the result to the configured base directory. Rejects absolute targets,
    /// `..` traversal, and symlink escapes that leave the base tree.
    fn resolve_path(
        &self,
        filename: &str,
        span: Span,
        stack: &[(PathBuf, u32)],
    ) -> Result<PathBuf, IncludeError> {
        let file_dir = self
            .file_map
            .get(span.file_id as usize)
            .and_then(|p| p.parent())
            .unwrap_or(&self.base_dir);
        let candidate = file_dir.join(filename);
        self.confine(candidate, filename, span, stack)
    }

    /// Reject a path that resolves outside [`Self::canonical_base`], returning the
    /// path to read or walk.
    ///
    /// An existing path is canonicalized (fully symlink-resolved) and checked
    /// against the base, so a later read cannot be redirected by swapping a
    /// symlink after the check. When canonicalization fails the path is checked
    /// lexically instead, which distinguishes a traversal attempt (reported as
    /// confinement) from an in-base path that simply does not exist (returned so
    /// the caller can treat a missing file as not-found and a missing directory
    /// as empty, matching Home Assistant). The one case that is refused outright
    /// is a *symlink* that does not canonicalize: its target is dangling or
    /// untrusted, and re-following it at read time could escape the base after
    /// this check (the TOCTOU vector), so it is reported as not-found rather than
    /// opened.
    fn confine(
        &self,
        candidate: PathBuf,
        filename: &str,
        span: Span,
        stack: &[(PathBuf, u32)],
    ) -> Result<PathBuf, IncludeError> {
        let confinement = || IncludeError {
            kind: IncludeErrorKind::Confinement,
            message: format!("'{filename}' resolves outside the configured include directory"),
            path: self.resolve_path_from_span(span),
            include_stack: stack.to_vec(),
            span: None,
        };

        match dunce::canonicalize(&candidate) {
            Ok(resolved) => {
                if !resolved.starts_with(&self.canonical_base) {
                    return Err(confinement());
                }
                Ok(resolved)
            }
            Err(_) => {
                // The path does not canonicalize (it is missing, or a dangling
                // symlink), so it is checked lexically. `canonical_base` is
                // absolute, so a relative candidate must be anchored to the
                // current directory first (the same directory `canonical_base`
                // was resolved against). Otherwise a relative `include_dir` like
                // "." leaves the candidate relative, and a missing in-tree file
                // (`missing.yaml`) fails `starts_with` and is misreported as a
                // confinement escape instead of a plain not-found.
                let absolute = if candidate.is_absolute() {
                    candidate.clone()
                } else {
                    std::env::current_dir()
                        .map(|cwd| cwd.join(&candidate))
                        .unwrap_or_else(|_| candidate.clone())
                };
                let lexical = lexical_normalize(&absolute);
                if !lexical.starts_with(&self.canonical_base) {
                    return Err(confinement());
                }
                // A symlink whose target does not canonicalize must not be
                // returned: `read_to_string` would re-follow it, and the target
                // could be swapped to point outside the base after this check.
                // `symlink_metadata` does not follow the final component, so it
                // detects exactly this dangling-symlink case.
                let is_symlink = candidate
                    .symlink_metadata()
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                if is_symlink {
                    return Err(IncludeError {
                        kind: IncludeErrorKind::NotFound,
                        message: format!("cannot read included file '{filename}': not found"),
                        path: self.resolve_path_from_span(span),
                        include_stack: stack.to_vec(),
                        span: None,
                    });
                }
                // A genuinely-missing, non-symlink path: safe to return. Opening
                // it as a file fails cleanly (not-found); a directory include
                // finds nothing (`is_dir()` is false) and resolves to empty.
                Ok(lexical)
            }
        }
    }

    fn resolve_path_from_span(&self, span: Span) -> PathBuf {
        self.file_map
            .get(span.file_id as usize)
            .cloned()
            .unwrap_or_else(|| self.base_dir.clone())
    }

    /// Read a confined file, register it for write-back, and compose it to AST
    /// nodes. These three steps always occur together at an include boundary;
    /// `display` names the file in the read error.
    fn read_and_compose(
        &mut self,
        path: &Path,
        display: &str,
        stack: &[(PathBuf, u32)],
    ) -> Result<(u32, Vec<YamlNode>), IncludeError> {
        let content = std::fs::read_to_string(path).map_err(|e| IncludeError {
            kind: IncludeErrorKind::NotFound,
            message: format!("cannot read included file '{display}': {e}"),
            path: path.to_path_buf(),
            include_stack: stack.to_vec(),
            span: None,
        })?;
        let file_id = self.register_file(path.to_path_buf(), Some(content.clone()));
        let nodes = compose_with_file_id(&content, file_id).map_err(|e| IncludeError {
            kind: IncludeErrorKind::Parse,
            message: e.message,
            path: path.to_path_buf(),
            include_stack: stack.to_vec(),
            span: Some(e.span),
        })?;
        Ok((file_id, nodes))
    }

    /// Extract the non-empty string argument of an include tag, or a structured
    /// "needs an argument" error naming `tag`. A missing argument (the tag with
    /// no scalar) and a blank or whitespace-only argument are both rejected, so
    /// `!include` / `!include_dir_*` always carry a real target.
    fn scalar_arg(
        &self,
        node: &YamlNode,
        tag: &str,
        stack: &[(PathBuf, u32)],
    ) -> Result<String, IncludeError> {
        match &node.kind {
            YamlNodeKind::Scalar(s, _) if !s.trim().is_empty() => Ok(s.clone()),
            _ => Err(IncludeError {
                kind: IncludeErrorKind::Invalid,
                message: format!("{tag} needs an argument"),
                path: self.resolve_path_from_span(node.span),
                include_stack: stack.to_vec(),
                span: None,
            }),
        }
    }
}

/// Normalize a path lexically (without touching the filesystem): collapse `.`
/// and resolve `..` by popping the previous component. Used for confinement
/// checks on paths that do not yet exist.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Extract the scalar text of a node (the argument of an include directive).
fn scalar_text(node: &YamlNode) -> Option<String> {
    match &node.kind {
        YamlNodeKind::Scalar(s, _) => Some(s.clone()),
        _ => None,
    }
}

/// Compute the write-back changes for a resolved include tree.
///
/// Returns a map of source-file path → new file contents. Each included file
/// is re-emitted from its resolved subtree, with any nested includes restored
/// to their `!include` directive form. The root file is intentionally omitted:
/// callers retain the original root document, and only modified includes need
/// to be written.
pub fn compute_include_changes(
    nodes: &[YamlNode],
    file_map: &[PathBuf],
    file_sources: &[Option<String>],
) -> Result<HashMap<PathBuf, Vec<u8>>, IncludeError> {
    let mut changes: HashMap<PathBuf, Vec<u8>> = HashMap::new();
    for (file_id, bytes) in crate::roundtrip::emit::collect_include_changes(nodes, file_sources) {
        if let Some(path) = file_map.get(file_id as usize) {
            // The same physical file can be included more than once. If two of its
            // occurrences re-emit to different bytes (one was edited, the other
            // not, or both edited differently), there is no single correct content
            // to write: refuse rather than silently dropping one edit.
            if let Some(existing) = changes.get(path) {
                if *existing != bytes {
                    return Err(IncludeError {
                        kind: IncludeErrorKind::Invalid,
                        message: format!(
                            "'{}' is included more than once with conflicting edits; \
                             write-back cannot store two different versions of one file",
                            path.display()
                        ),
                        path: path.clone(),
                        include_stack: Vec::new(),
                        span: None,
                    });
                }
            }
            changes.insert(path.clone(), bytes);
        }
    }
    Ok(changes)
}

/// What kind of include-resolution failure occurred, so the FFI layer can raise
/// a precise Python exception (e.g. a missing file versus an escape attempt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IncludeErrorKind {
    /// An included file or directory does not exist or could not be read.
    NotFound,
    /// An include refers back to a file already in the chain.
    Circular,
    /// The include chain grew past [`MAX_INCLUDE_DEPTH`].
    Depth,
    /// A target resolves outside the configured base directory.
    Confinement,
    /// A `!secret` name is not defined in any `secrets.yaml`.
    SecretNotFound,
    /// An `!env_var` references an undefined variable with no default.
    EnvVarUndefined,
    /// An included file is not valid YAML (the composer rejected it). Carries the
    /// failing span so it surfaces as a located `YAMLRocksParseError` pointing at
    /// the included file, identical to how the same content errors as a root.
    Parse,
    /// A malformed argument or other resolution failure. The catch-all.
    #[default]
    Invalid,
}

/// Error during include resolution.
#[derive(Debug)]
pub struct IncludeError {
    pub kind: IncludeErrorKind,
    pub message: String,
    pub path: PathBuf,
    pub include_stack: Vec<(PathBuf, u32)>,
    /// The source span of the offending directive, when known, so the Python
    /// error can carry `.line`/`.column` (not just `.file`). Populated for the
    /// `!secret`/`!env_var` failures that point at a specific requesting node.
    pub span: Option<Span>,
}

impl std::fmt::Display for IncludeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} in {}", self.message, self.path.display())?;
        for (path, line) in &self.include_stack {
            write!(f, "\n  included from {}, line {}", path.display(), line + 1)?;
        }
        Ok(())
    }
}

impl std::error::Error for IncludeError {}

#[cfg(all(test, unix))]
mod tests {
    use super::lexical_normalize;
    use std::path::Path;

    fn norm(path: &str) -> String {
        lexical_normalize(Path::new(path))
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn collapses_current_dir_segments() {
        assert_eq!(norm("config/./sub/file.yaml"), "config/sub/file.yaml");
    }

    #[test]
    fn resolves_parent_dir_segments() {
        assert_eq!(norm("config/sub/../other.yaml"), "config/other.yaml");
    }

    #[test]
    fn leaves_plain_relative_paths_untouched() {
        assert_eq!(norm("a/b/c.yaml"), "a/b/c.yaml");
    }

    #[test]
    fn dotdot_escape_is_surfaced_for_confinement_to_reject() {
        // Lexical normalization does not keep a leading `..` pair, so an escape
        // collapses to a path that no longer starts with the base directory;
        // confine() then rejects it.
        assert_eq!(norm("base/../../etc/passwd"), "etc/passwd");
    }

    #[test]
    fn preserves_absolute_root() {
        assert_eq!(norm("/srv/./config/app.yaml"), "/srv/config/app.yaml");
    }
}
