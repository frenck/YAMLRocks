mod yaml11;
mod yaml12;

pub use yaml11::Yaml11Resolver;
pub use yaml12::Yaml12Resolver;

use crate::scanner::ScalarStyle;

/// Which scalar-resolution schema to apply. Carried (instead of a bare
/// `yaml_11` bool) so the PyYAML-compat variant travels everywhere 1.1 is
/// interpreted: scalar resolution, annotation, the 1.1-vs-1.2 migration
/// diagnostic, and the upgrade rewrite.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Schema {
    /// YAML 1.2 core schema (the default).
    #[default]
    Yaml12,
    /// The full YAML 1.1 schema, including bare `y`/`n` booleans (spec-correct).
    Yaml11,
    /// YAML 1.1 with PyYAML's deliberately off-spec bool set (no bare `y`/`n`),
    /// for interop with the PyYAML-based ecosystem (Home Assistant, ESPHoME,
    /// Ansible).
    Yaml11PyYaml,
}

impl Schema {
    /// Build a schema from the two option flags. PyYAML-compat implies 1.1, so
    /// `OPT_PYYAML_COMPAT` works on its own.
    pub fn new(yaml_11: bool, pyyaml_compat: bool) -> Self {
        if pyyaml_compat {
            Schema::Yaml11PyYaml
        } else if yaml_11 {
            Schema::Yaml11
        } else {
            Schema::Yaml12
        }
    }

    /// Whether this is one of the YAML 1.1 variants (spec or PyYAML).
    pub fn is_yaml_11(self) -> bool {
        !matches!(self, Schema::Yaml12)
    }

    /// Classify a scalar under this schema (no string allocation).
    pub fn classify(self, value: &str, style: ScalarStyle, tag: Option<&str>) -> ScalarKind {
        match self {
            Schema::Yaml12 => Yaml12Resolver.classify(value, style, tag),
            Schema::Yaml11 => Yaml11Resolver::default().classify(value, style, tag),
            Schema::Yaml11PyYaml => Yaml11Resolver {
                pyyaml_compat: true,
            }
            .classify(value, style, tag),
        }
    }

    /// Resolve a scalar to an owned value under this schema.
    pub fn resolve(self, value: &str, style: ScalarStyle, tag: Option<&str>) -> ResolvedValue {
        match self {
            Schema::Yaml12 => Yaml12Resolver.resolve(value, style, tag),
            Schema::Yaml11 => Yaml11Resolver::default().resolve(value, style, tag),
            Schema::Yaml11PyYaml => Yaml11Resolver {
                pyyaml_compat: true,
            }
            .resolve(value, style, tag),
        }
    }
}

/// Resolved type of a YAML scalar, owning a copy of the text for the string
/// case.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedValue {
    Null,
    Bool(bool),
    Int(i64),
    /// An integer too large for `i64`, as its exact decimal text.
    BigInt(String),
    Float(f64),
    String(String),
}

/// Scalar type classification that carries no owned string.
///
/// Where the caller already owns the source text (the fast-path decoder owns
/// the scanner's scalar `String`), classifying without re-allocating lets it
/// move that text straight into the result instead of cloning it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScalarKind {
    Null,
    Bool(bool),
    Int(i64),
    /// An integer whose magnitude exceeds `i64`. Carries no value: the caller
    /// owns the source text and reads the exact digits from there.
    BigInt,
    Float(f64),
    /// The scalar is a string; its value is the (already owned) source text.
    Str,
    /// An explicit `!!merge` tag: the value is the literal merge key `<<`,
    /// independent of the source text.
    Merge,
}

/// Trait for resolving plain scalar values to typed values.
///
/// This is the only component that differs between YAML 1.1 and 1.2.
pub trait Resolver {
    /// Classify a scalar's type without allocating for the string case.
    fn classify(&self, value: &str, style: ScalarStyle, tag: Option<&str>) -> ScalarKind;

    /// Resolve a scalar to an owned value. Used where the caller does not own
    /// the source text (the annotated AST path); the fast-path decoder uses
    /// [`Resolver::classify`] instead to avoid the string copy.
    fn resolve(&self, value: &str, style: ScalarStyle, tag: Option<&str>) -> ResolvedValue {
        match self.classify(value, style, tag) {
            ScalarKind::Null => ResolvedValue::Null,
            ScalarKind::Bool(b) => ResolvedValue::Bool(b),
            ScalarKind::Int(i) => ResolvedValue::Int(i),
            ScalarKind::BigInt => ResolvedValue::BigInt(value.to_owned()),
            ScalarKind::Float(f) => ResolvedValue::Float(f),
            ScalarKind::Str => ResolvedValue::String(value.to_owned()),
            ScalarKind::Merge => ResolvedValue::String("<<".to_owned()),
        }
    }
}

/// A short type name for a [`ScalarKind`], for diagnostics.
fn kind_name(kind: ScalarKind) -> &'static str {
    match kind {
        ScalarKind::Null => "null",
        ScalarKind::Bool(_) => "bool",
        ScalarKind::Int(_) | ScalarKind::BigInt => "int",
        ScalarKind::Float(_) => "float",
        ScalarKind::Str => "str",
        ScalarKind::Merge => "merge",
    }
}

/// When a plain scalar's resolved type differs between the YAML 1.1 and 1.2
/// schemas, return `(yaml_1_1_type, yaml_1_2_type)` short type names; otherwise
/// `None`.
///
/// Only divergences where 1.1 assigns a *non-string* type are reported, since
/// those are the 1.1-only constructs (yes/no booleans, `0777` octals,
/// sexagesimals, underscore digit groups, ...) that a migration to 1.2 needs to
/// find: each would silently become a plain string under 1.2. Quoted and tagged
/// scalars resolve identically under both schemas, so they never diverge here.
pub fn yaml_11_divergence(
    schema: Schema,
    value: &str,
    style: ScalarStyle,
    tag: Option<&str>,
) -> Option<(&'static str, &'static str)> {
    // `schema` is the active 1.1 reading (spec or PyYAML), compared against 1.2.
    // Under PyYAML-compat, `y`/`n` are strings in both, so they stop diverging.
    let n11 = kind_name(schema.classify(value, style, tag));
    let n12 = kind_name(Schema::Yaml12.classify(value, style, tag));
    (n11 != n12 && n11 != "str").then_some((n11, n12))
}

// Atoms shared by the YAML 1.1 and 1.2 resolvers. The schemas diverge on bools,
// octals, underscores, and sexagesimals, but agree on these.

/// Whether a plain scalar is YAML null: empty, or one of the null literals.
fn is_null(value: &str) -> bool {
    value.is_empty() || matches!(value, "~" | "null" | "Null" | "NULL")
}

/// Parse the special float spellings shared by both schemas
/// (`.inf`/`+.inf`/`-.inf`/`.nan`). Returns `None` for anything else.
fn parse_special_float(value: &str) -> Option<f64> {
    match value {
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => Some(f64::INFINITY),
        "-.inf" | "-.Inf" | "-.INF" => Some(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => Some(f64::NAN),
        _ => None,
    }
}

/// Split an optional leading `+`/`-` from a numeric scalar, returning
/// `(is_negative, rest)`. `None` if the value is empty or only a sign.
fn split_sign(value: &str) -> Option<(bool, &str)> {
    let (negative, rest) = match value.as_bytes().first()? {
        b'-' => (true, &value[1..]),
        b'+' => (false, &value[1..]),
        _ => (false, value),
    };
    if rest.is_empty() {
        None
    } else {
        Some((negative, rest))
    }
}

/// Parse a radix-prefixed integer body (the part after `0x`/`0o`/`0b`/`0`) that
/// must be unsigned. `i64::from_str_radix` itself accepts a leading `+`/`-`, so a
/// body like `-5` (from `0x-5`) would wrongly read as a negative integer; the
/// sign is only valid before the prefix, handled by [`split_sign`].
fn from_radix_unsigned(body: &str, radix: u32) -> Option<i64> {
    if body.starts_with(['+', '-']) {
        return None;
    }
    i64::from_str_radix(body, radix).ok()
}

/// Whether `value` is a base-10 integer literal (optional sign, then digits with
/// no leading zero unless the value is `0`). Used to recognize an integer whose
/// magnitude overflows `i64` so it resolves as a big integer rather than a string.
/// Shared by the 1.1 and 1.2 schemas, which agree on plain decimal integers.
pub(crate) fn is_big_decimal_int(value: &str) -> bool {
    let Some((_, rest)) = split_sign(value) else {
        return false;
    };
    rest.bytes().all(|b| b.is_ascii_digit()) && (rest == "0" || !rest.starts_with('0'))
}
