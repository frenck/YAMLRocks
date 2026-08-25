use crate::scanner::{ScalarStyle, Span};

/// A rich YAML node that preserves all structural information for round-tripping.
#[derive(Debug, Clone)]
pub struct YamlNode {
    pub kind: YamlNodeKind,
    pub span: Span,
    /// Byte offset just past this node's source content (the exact end). For a
    /// scalar it is the scanner's recorded content end (past the closing quote, or
    /// the last plain/block-scalar character); for a collection it is the furthest
    /// end of any child; for an empty or synthetic node it equals `span.offset`.
    /// Lets a consumer slice the exact source bytes of a node (`src[offset..end]`),
    /// which line/column alone cannot do.
    pub end_offset: usize,
    /// The exact 0-based source position just past this node's last character,
    /// derived from [`end_offset`](Self::end_offset) against the source. Unlike
    /// [`end`](Self::end), this is exact for quoted and escaped scalars (it lands
    /// past the closing quote, not at the unescaped value's length). The composer
    /// fills it in; synthetic (edited) nodes keep the `span` start as a default.
    pub end_line: u32,
    /// The exact 0-based source column just past this node's last character. See
    /// [`end_line`](Self::end_line).
    pub end_column: u32,
    pub anchor: Option<String>,
    pub tag: Option<String>,
    pub comments: Comments,
    pub style: NodeStyle,
    /// If this node was produced by resolving an `!include` directive, records
    /// the original directive so it can be restored when writing the file that
    /// contained it. `None` for ordinary nodes.
    pub source: Option<IncludeSource>,
    /// Whether this document's source began with an explicit `---` marker. Only
    /// meaningful on a document root node; `false` for nested nodes. Preserved so
    /// re-emission (after an edit, or when writing an included file) keeps the
    /// marker instead of silently dropping it.
    pub explicit_start: bool,
    /// The `%YAML`/`%TAG` directives that introduced this document, in source
    /// order and without their leading `%` (`TAG !e! tag:example.com,2020:`).
    /// Only meaningful on a document root node; empty for nested nodes. Preserved
    /// so re-emission after an edit keeps a `%TAG` handle in scope (dropping it
    /// would leave a `!e!foo` node with an undefined handle that no longer
    /// reloads).
    pub directives: Vec<String>,
    /// Whether the stream began with a UTF-8 byte order mark. Only meaningful on
    /// the first document root; `false` otherwise. Preserved so re-emission keeps
    /// the mark and a round-trip stays byte-for-byte.
    pub leading_bom: bool,
    /// Whether this node was created from a Python value (an edit) rather than
    /// parsed from source. A *synthetic* null follows the document's null style on
    /// re-emission; a *loaded* null always re-emits in its original form so an
    /// untouched value stays byte-for-byte.
    pub synthetic: bool,
    /// Whether this node, as a mapping key, was written with an explicit `?`
    /// indicator in the source (`? key`). Only meaningful when the node sits in
    /// key position; `false` otherwise. Preserved so re-emission after an edit
    /// keeps the author's explicit-key form instead of collapsing it to the
    /// implicit `key:` form. A key that structurally *requires* the explicit form
    /// (a block collection or block scalar) is emitted explicitly regardless.
    pub explicit_key: bool,
}

impl YamlNode {
    pub fn new(kind: YamlNodeKind, span: Span) -> Self {
        Self {
            kind,
            // Default to a zero-width extent at the start; the composer overwrites
            // this with the real source end for parsed nodes.
            end_offset: span.offset,
            end_line: span.line,
            end_column: span.column,
            span,
            anchor: None,
            tag: None,
            comments: Comments::default(),
            style: NodeStyle::Block,
            source: None,
            explicit_start: false,
            directives: Vec::new(),
            leading_bom: false,
            synthetic: false,
            explicit_key: false,
        }
    }

    pub fn with_anchor(mut self, anchor: Option<String>) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn with_tag(mut self, tag: Option<String>) -> Self {
        self.tag = tag;
        self
    }

    pub fn with_comments(mut self, comments: Comments) -> Self {
        self.comments = comments;
        self
    }

    pub fn with_style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Mark this node as modified (for diff-based emission).
    pub fn mark_modified(&mut self) {
        self.comments.modified = true;
    }

    /// The node's 0-based end position `(line, column)`: the point just past its
    /// last character, mirroring PyYAML's `end_mark`. For a scalar this is its
    /// start plus the text length (accounting for embedded newlines); for a
    /// collection it is the furthest end of any child (the end of the block).
    /// Empty collections, aliases, and null nodes report their own start.
    ///
    /// The column is derived from the (post-scan) scalar text, so for a quoted or
    /// escaped scalar it is an approximation of the source span rather than the
    /// exact byte position of the closing quote. It is kept for the internal
    /// comment-attachment heuristic, which is calibrated against this basis. The
    /// exact source position is available as [`end_line`](Self::end_line) /
    /// [`end_column`](Self::end_column), which is what the round-trip
    /// `YAMLRocksNode.range()` and the annotated types expose.
    pub fn end(&self) -> (u32, u32) {
        match &self.kind {
            YamlNodeKind::Scalar(text, _) => {
                let newlines = text.matches('\n').count() as u32;
                if newlines == 0 {
                    (
                        self.span.line,
                        self.span.column + text.chars().count() as u32,
                    )
                } else {
                    let last = text.rsplit('\n').next().unwrap_or("");
                    (self.span.line + newlines, last.chars().count() as u32)
                }
            }
            YamlNodeKind::Mapping(pairs) => pairs
                .iter()
                .map(|(k, v)| k.end().max(v.end()))
                .max()
                .unwrap_or((self.span.line, self.span.column)),
            YamlNodeKind::Sequence(items) => items
                .iter()
                .map(YamlNode::end)
                .max()
                .unwrap_or((self.span.line, self.span.column)),
            _ => (self.span.line, self.span.column),
        }
    }

    /// The tag that produced this node, for provenance. A config-tag directive
    /// (`!secret`/`!env_var`/`!include*`, recorded by include resolution) takes
    /// priority; otherwise the node's own *custom* application tag (`!mytag`).
    /// Returns `None` for an untagged node and for core `!!type` tags, which are
    /// type coercion already reflected in the value rather than provenance.
    pub fn source_tag(&self) -> Option<&str> {
        if let Some(source) = &self.source {
            return Some(&source.tag);
        }
        match self.tag.as_deref() {
            Some(tag) if crate::decode::is_custom_tag(tag) => Some(tag),
            _ => None,
        }
    }

    /// The argument of the config directive that produced this node: the secret
    /// name for `!secret NAME`, the path for `!include PATH`, or the variable
    /// spec for `!env_var NAME [default]`. `None` when there is no such directive
    /// (an inline scalar, or a custom tag, whose content is the value itself).
    /// Pairs with [`source_tag`](Self::source_tag) to reconstruct the directive.
    pub fn source_target(&self) -> Option<&str> {
        self.source.as_ref().map(|source| source.target.as_str())
    }
}

/// Whether `tag` is one of the five built-in `!include` family directives, used
/// to derive the `is_include` provenance predicate.
pub fn is_include_tag(tag: Option<&str>) -> bool {
    matches!(
        tag,
        Some(
            "!include"
                | "!include_dir_list"
                | "!include_dir_named"
                | "!include_dir_merge_list"
                | "!include_dir_merge_named"
        )
    )
}

/// The content of a YAML node.
#[derive(Debug, Clone)]
pub enum YamlNodeKind {
    Scalar(String, ScalarStyle),
    Sequence(Vec<YamlNode>),
    Mapping(Vec<(YamlNode, YamlNode)>),
    Alias(String),
    Null,
}

/// Comments attached to a node.
#[derive(Debug, Clone, Default)]
pub struct Comments {
    /// Number of blank source lines immediately before this node's leading block
    /// (its head comments, or the node itself when it has none). Preserved so a
    /// re-emitted document keeps the author's section spacing. Computed from the
    /// original spans at compose time, so it survives the stale spans a modified
    /// document carries; a synthetic (edited-in) node defaults to `0`.
    pub blank_before: u32,
    /// Comments on their own line(s) before this node.
    pub head: Vec<HeadComment>,
    /// Comment on the same line after this node's value.
    pub inline: Option<String>,
    /// Number of spaces between the value and the `#` of [`inline`](Self::inline),
    /// preserving alignment padding (`x: 1      # note`) across a re-emit. `0`
    /// means "not captured" (a synthetic comment), which emits as a single space.
    pub inline_spaces: u32,
    /// Whether [`inline`](Self::inline) was written on the line that *introduces*
    /// this value (after a mapping key's `:` or a sequence `-`) while the value
    /// itself begins on a later line, as in `key: # note`. The emitter keeps such
    /// a comment on the introducer line instead of moving it past the value.
    /// `false` for the usual trailing comment (`key: value # note`), which the
    /// emitter writes after the value.
    pub inline_before_value: bool,
    /// Number of spaces between this node's introducer (a mapping key's `:` or a
    /// sequence `-`) and an inline value sharing its line, preserving alignment
    /// like `example:      true` or `-    item`. `0` means "not captured" (a
    /// synthetic or block-positioned value), which emits as a single space.
    pub value_pad: u32,
    /// For a block sequence that is itself a sequence item: whether it was
    /// written compactly on its parent's dash line (`- - 1`) rather than broken
    /// to the next line. Preserved so the compact layout survives a re-emit.
    pub compact: bool,
    /// The verbatim source of a block scalar (`|`/`>`), from the indicator
    /// through its last content line. Folding a `>` block loses the author's
    /// line breaks, so the emitter cannot reconstruct it; it replays this slice
    /// instead. `None` for every other node and for an edited (synthetic) scalar.
    pub raw: Option<String>,
    /// Comments after a blank line following this node.
    pub foot: Vec<String>,
    /// Whether this node was modified since loading (for diff-based emission).
    pub modified: bool,
}

/// A standalone comment line above a node, and which side of the node's
/// introducer it was written on.
///
/// A section comment can sit above a `-` while the dash carries its own comment
/// (`- # note`) and further comments follow underneath. They reach the node as
/// one run, so each has to remember its own side to be re-emitted there.
#[derive(Debug, Clone)]
pub struct HeadComment {
    /// The comment text, without the leading `#`. A `Box<str>` rather than a
    /// `String`: the text never grows after attachment, and dropping the unused
    /// capacity keeps this the same size the plain `String` was, so recording
    /// the side costs nothing.
    pub text: Box<str>,
    /// Whether it was written *below* the comment on the node's introducer,
    /// rather than above the introducer itself. Always `false` for a node whose
    /// introducer carries no comment, where there is no dividing line.
    pub below_introducer: bool,
}

impl HeadComment {
    /// A comment above the node's introducer, the position every comment takes
    /// when nothing divides the block.
    pub fn above(text: impl Into<Box<str>>) -> Self {
        Self {
            text: text.into(),
            below_introducer: false,
        }
    }
}

impl Comments {
    /// The head comments written above this node's introducer (a sequence `-`).
    /// Everything, for a node whose introducer carries no comment of its own.
    pub fn head_above_introducer(&self) -> impl Iterator<Item = &str> {
        self.head
            .iter()
            .filter(|comment| !comment.below_introducer)
            .map(|comment| &*comment.text)
    }

    /// The head comments written below that introducer, between its own comment
    /// and the node's first line.
    pub fn head_below_introducer(&self) -> impl Iterator<Item = &str> {
        self.head
            .iter()
            .filter(|comment| comment.below_introducer)
            .map(|comment| &*comment.text)
    }
}

/// The presentation style of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStyle {
    Block,
    Flow,
}

/// Records the `!include`-style directive that produced a resolved node, so the
/// directive can be re-emitted into its original file during write-back.
#[derive(Debug, Clone)]
pub struct IncludeSource {
    /// The include tag, e.g. `!include` or `!include_dir_list`.
    pub tag: String,
    /// The argument of the directive, e.g. `automations.yaml`.
    pub target: String,
    /// The `file_id` of the file the resolved content was loaded from. For
    /// directory includes (which span multiple files) this is `None`.
    pub target_file_id: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::{Comments, HeadComment, NodeStyle, YamlNode, YamlNodeKind};
    use crate::scanner::{ScalarStyle, Span};

    fn span() -> Span {
        Span {
            file_id: 0,
            line: 0,
            column: 0,
            offset: 0,
        }
    }

    #[test]
    fn new_node_has_block_style_and_no_metadata() {
        let node = YamlNode::new(YamlNodeKind::Null, span());
        assert_eq!(node.style, NodeStyle::Block);
        assert!(node.anchor.is_none());
        assert!(node.tag.is_none());
        assert!(node.source.is_none());
        assert!(!node.explicit_start);
        assert!(!node.comments.modified);
    }

    #[test]
    fn builders_set_each_field() {
        let comments = Comments {
            head: vec![HeadComment::above("note")],
            ..Default::default()
        };
        let node = YamlNode::new(
            YamlNodeKind::Scalar("v".to_owned(), ScalarStyle::Plain),
            span(),
        )
        .with_anchor(Some("a".to_owned()))
        .with_tag(Some("!t".to_owned()))
        .with_style(NodeStyle::Flow)
        .with_comments(comments);
        assert_eq!(node.anchor.as_deref(), Some("a"));
        assert_eq!(node.tag.as_deref(), Some("!t"));
        assert_eq!(node.style, NodeStyle::Flow);
        assert_eq!(node.comments.head[0].text.as_ref(), "note");
        assert!(matches!(node.kind, YamlNodeKind::Scalar(..)));
    }

    #[test]
    fn head_comments_split_around_their_introducer() {
        // A section comment above the `-`, then two written below its comment.
        let comments = Comments {
            head: vec![
                HeadComment::above("above"),
                HeadComment {
                    text: "below".into(),
                    below_introducer: true,
                },
                HeadComment {
                    text: "further".into(),
                    below_introducer: true,
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            comments.head_above_introducer().collect::<Vec<_>>(),
            ["above"]
        );
        assert_eq!(
            comments.head_below_introducer().collect::<Vec<_>>(),
            ["below", "further"]
        );
    }

    #[test]
    fn mark_modified_sets_the_flag() {
        let mut node = YamlNode::new(YamlNodeKind::Null, span());
        node.mark_modified();
        assert!(node.comments.modified);
    }
}
