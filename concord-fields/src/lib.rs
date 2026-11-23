//! The one parser of `fields.toml`, the shared field-name registry. The
//! `Event` derive (never-log names at compile time) and `concord-lint` (every
//! rule that reads the registry) both depend on it, so a layout change cannot
//! make one of them stop enforcing what the other still does.

mod registry;

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

pub use registry::{label_names, FieldVerdict, Registry};

/// The registry shared by every repo, embedded at build time.
pub const SHARED: &str = include_str!("../../fields.toml");

/// The attribute prefix under which concord-telemetry stamps labels.
pub const LABEL_PREFIX: &str = "concord.label.";

/// The namespace reserved for concord-telemetry's own keys (`concord.label.*`,
/// `concord.label_conflicts`, ...). No field or baggage key may be in it.
pub const RESERVED_NAMESPACE: &str = "concord.";

/// The longest value of a free-text field, in bytes.
pub const FREE_TEXT_MAX: usize = 1024;

/// The type a registered field carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
    Str,
    Int,
    Float,
    Bool,
}

/// One registered field.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FieldDef {
    pub meaning: String,
    pub ty: FieldType,
    /// `label = true`: the field is carried as baggage (a label), and must not
    /// be recorded through `Debug`.
    pub label: bool,
    /// `free_text = true`: a string whose value may be up to `FREE_TEXT_MAX`.
    pub free_text: bool,
}

/// One registered cause kind: the label that carries its id, and optionally the
/// caller label that carries a caller's id the receiver would overwrite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CauseDef {
    pub label: String,
    pub caller_label: Option<String>,
}

/// A parsed registry file.
#[derive(Clone, Debug, Default)]
pub struct Parsed {
    pub names: BTreeMap<String, FieldDef>,
    pub never: BTreeSet<String>,
    /// `[lint] log_macros`: a repo's own wrapper macros (bare names) that the
    /// lint treats as field-bearing: first argument the error, the rest fields.
    pub log_macros: BTreeSet<String>,
    /// `[causes]`: cause kind → the labels that carry its id.
    pub causes: BTreeMap<String, CauseDef>,
}

/// Why a registry file was refused.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("{0}")]
    Toml(#[from] toml::de::Error),
    #[error("{name} is in the reserved namespace concord.")]
    ReservedNamespace { name: String },
    #[error("{name} has no type (expected str, int, float or bool)")]
    MissingType { name: String },
    #[error("{name} has unknown type `{found}` (expected str, int, float or bool)")]
    UnknownType { name: String, found: String },
    #[error("{name} is a label, so its type must be str")]
    LabelNotStr { name: String },
    #[error("{name} cannot be both a label and free_text")]
    LabelAndFreeText { name: String },
    #[error("[lint] log_macros entry `{name}` must be a bare macro name other than a built-in error or tracing macro")]
    BadLogMacro { name: String },
    #[error("{name} shadows a shared name")]
    Shadows { name: String },
    #[error("{name} is registered and never-log")]
    NeverAndRegistered { name: String },
    #[error("{name} is a never-log name")]
    NeverName { name: String },
    #[error("cause kind `{kind}` must be a lowercase word (^[a-z][a-z_]*$)")]
    CauseKindName { kind: String },
    #[error("cause kind `{kind}` names `{label}`, which is not a label in [fields]")]
    CauseLabelNotALabel { kind: String, label: String },
    #[error("`{label}` serves more than one cause kind")]
    CauseLabelTwice { label: String },
    #[error("a local registry may not have a [causes] table")]
    LocalCauses,
}

#[derive(Default, Deserialize)]
struct Lint {
    #[serde(default)]
    log_macros: Vec<String>,
}

#[derive(Deserialize)]
struct Document {
    #[serde(default)]
    lint: Lint,
    #[serde(default)]
    fields: BTreeMap<String, Definition>,
    #[serde(default)]
    never: Never,
    #[serde(default)]
    causes: BTreeMap<String, CauseRow>,
}

#[derive(Deserialize)]
struct Definition {
    meaning: String,
    #[serde(default)]
    #[serde(rename = "type")]
    ty: Option<String>,
    #[serde(default)]
    label: bool,
    #[serde(default)]
    free_text: bool,
}

#[derive(Default, Deserialize)]
struct Never {
    #[serde(default)]
    names: Vec<String>,
}

#[derive(Deserialize)]
struct CauseRow {
    label: String,
    #[serde(default)]
    caller_label: Option<String>,
}

/// The cause labels of a parsed registry as `(label, kind)`, sorted by label.
pub fn cause_labels(parsed: &Parsed) -> Vec<(String, String)> {
    let mut labels: Vec<(String, String)> = parsed
        .causes
        .iter()
        .flat_map(|(kind, cause)| {
            let kind = kind.clone();
            std::iter::once((cause.label.clone(), kind.clone())).chain(
                cause
                    .caller_label
                    .clone()
                    .map(|caller| (caller, kind.clone())),
            )
        })
        .collect();
    labels.sort();
    labels
}

fn field_type(name: &str, found: Option<String>) -> Result<FieldType, ParseError> {
    match found.as_deref() {
        None => Err(ParseError::MissingType {
            name: name.to_string(),
        }),
        Some("str") => Ok(FieldType::Str),
        Some("int") => Ok(FieldType::Int),
        Some("float") => Ok(FieldType::Float),
        Some("bool") => Ok(FieldType::Bool),
        Some(other) => Err(ParseError::UnknownType {
            name: name.to_string(),
            found: other.to_string(),
        }),
    }
}

/// Parses a registry file. A field in the reserved `concord.` namespace is an
/// error.
pub fn parse(source: &str) -> Result<Parsed, ParseError> {
    let document: Document = toml::from_str(source)?;
    let mut names = BTreeMap::new();
    for (name, definition) in document.fields {
        if name.starts_with(RESERVED_NAMESPACE) {
            return Err(ParseError::ReservedNamespace { name });
        }
        let ty = field_type(&name, definition.ty)?;
        if definition.label && ty != FieldType::Str {
            return Err(ParseError::LabelNotStr { name });
        }
        if definition.label && definition.free_text {
            return Err(ParseError::LabelAndFreeText { name });
        }
        names.insert(
            name,
            FieldDef {
                meaning: definition.meaning,
                ty,
                label: definition.label,
                free_text: definition.free_text,
            },
        );
    }
    let mut log_macros = BTreeSet::new();
    for name in document.lint.log_macros {
        let bare = !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !bare
            || [
                "error",
                "error_anyhow",
                "warn_anyhow",
                "error_at",
                "error_at_anyhow",
                "trace",
                "debug",
                "info",
                "warn",
                "event",
                "span",
            ]
            .contains(&name.as_str())
        {
            return Err(ParseError::BadLogMacro { name });
        }
        log_macros.insert(name);
    }
    let mut causes = BTreeMap::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (kind, row) in document.causes {
        if !(kind.starts_with(|c: char| c.is_ascii_lowercase())
            && kind.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        {
            return Err(ParseError::CauseKindName { kind });
        }
        let labels = std::iter::once(&row.label).chain(row.caller_label.as_ref());
        for label in labels {
            if !names.get(label).is_some_and(|def| def.label) {
                return Err(ParseError::CauseLabelNotALabel {
                    kind: kind.clone(),
                    label: label.clone(),
                });
            }
            if !seen.insert(label.clone()) {
                return Err(ParseError::CauseLabelTwice {
                    label: label.clone(),
                });
            }
        }
        causes.insert(
            kind,
            CauseDef {
                label: row.label,
                caller_label: row.caller_label,
            },
        );
    }
    Ok(Parsed {
        names,
        never: document.never.names.into_iter().collect(),
        log_macros,
        causes,
    })
}

/// The shared registry, parsed.
pub fn shared() -> Parsed {
    parse(SHARED).expect("embedded registry is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multi_line_never_array_is_read() {
        let parsed =
            parse("[never]\nnames = [\n  \"content\",\n  \"token\", # why\n  \"secret\",\n]\n")
                .expect("parses");
        assert_eq!(
            parsed.never.into_iter().collect::<Vec<_>>(),
            ["content", "secret", "token"]
        );
    }

    #[test]
    fn the_label_flag_and_the_type_column() {
        let parsed = parse(
            "[fields]\naction_id = { type = \"str\", meaning = \"x\", label = true }\nname = { type = \"str\", meaning = \"y\" }\n",
        )
        .expect("parses");
        assert!(parsed.names["action_id"].label);
        assert!(!parsed.names["name"].label);
    }

    #[test]
    fn the_reserved_namespace_is_not_a_field_name() {
        for name in ["concord.label.x", "concord.trace.y", "concord.anything"] {
            let source = format!("[fields]\n\"{name}\" = {{ type = \"str\", meaning = \"x\" }}\n");
            let error = parse(&source).unwrap_err();
            assert!(error.to_string().contains("reserved"), "{error}");
        }
    }

    #[test]
    fn the_shared_registry_parses_and_marks_its_labels() {
        let shared = shared();
        for name in [
            "action_id",
            "agent_id",
            "daemon_id",
            "session_id",
            "turn_id",
            "delivery_id",
            "request_id",
            "message_id",
            "instance_id",
            "place_id",
            "source_instance_id",
            "source_turn_id",
        ] {
            assert!(shared.names[name].label, "{name}");
        }
        assert!(!shared.names["room_id"].label);
        assert!(!shared.names["causes"].label);
        assert!(shared.never.contains("password"));
    }
}
