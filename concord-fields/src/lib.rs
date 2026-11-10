//! The one parser of `fields.toml`, the shared field-name registry. The
//! `Event` derive (never-log names at compile time) and `concord-lint` (every
//! rule that reads the registry) both depend on it, so a layout change cannot
//! make one of them stop enforcing what the other still does.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// The registry shared by every repo, embedded at build time.
pub const SHARED: &str = include_str!("../../fields.toml");

/// The attribute prefix under which concord-telemetry stamps labels.
pub const LABEL_PREFIX: &str = "concord.label.";

/// The namespace reserved for concord-telemetry's own keys (`concord.label.*`,
/// `concord.label_conflicts`, ...). No field or baggage key may be in it.
pub const RESERVED_NAMESPACE: &str = "concord.";

/// One registered field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDef {
    pub meaning: String,
    /// `label = true`: the field is carried as baggage (a label), and must not
    /// be recorded through `Debug`.
    pub label: bool,
}

/// A parsed registry file.
#[derive(Clone, Debug, Default)]
pub struct Parsed {
    pub names: BTreeMap<String, FieldDef>,
    pub never: BTreeSet<String>,
}

#[derive(Deserialize)]
struct Document {
    #[serde(default)]
    fields: BTreeMap<String, Definition>,
    #[serde(default)]
    never: Never,
}

#[derive(Deserialize)]
struct Definition {
    meaning: String,
    #[serde(default)]
    label: bool,
}

#[derive(Default, Deserialize)]
struct Never {
    #[serde(default)]
    names: Vec<String>,
}

/// Parses a registry file. A field in the reserved `concord.` namespace is an
/// error.
pub fn parse(source: &str) -> Result<Parsed, String> {
    let document: Document = toml::from_str(source).map_err(|error| error.to_string())?;
    let mut names = BTreeMap::new();
    for (name, definition) in document.fields {
        if name.starts_with(RESERVED_NAMESPACE) {
            return Err(format!(
                "{name} is in the reserved namespace {RESERVED_NAMESPACE}"
            ));
        }
        names.insert(
            name,
            FieldDef {
                meaning: definition.meaning,
                label: definition.label,
            },
        );
    }
    Ok(Parsed {
        names,
        never: document.never.names.into_iter().collect(),
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
            "[fields]\naction_id = { meaning = \"x\", label = true }\nname = { type = \"str\", meaning = \"y\" }\n",
        )
        .expect("parses; the old type column is ignored");
        assert!(parsed.names["action_id"].label);
        assert!(!parsed.names["name"].label);
    }

    #[test]
    fn the_reserved_namespace_is_not_a_field_name() {
        for name in ["concord.label.x", "concord.trace.y", "concord.anything"] {
            let source = format!("[fields]\n\"{name}\" = {{ meaning = \"x\" }}\n");
            let error = parse(&source).unwrap_err();
            assert!(error.contains("reserved"), "{error}");
        }
    }

    #[test]
    fn the_shared_registry_parses_and_marks_its_labels() {
        let shared = shared();
        for name in [
            "action_id",
            "agent_id",
            "session_id",
            "turn_id",
            "delivery_id",
            "request_id",
            "message_id",
            "instance_id",
            "machine_id",
        ] {
            assert!(shared.names[name].label, "{name}");
        }
        assert!(!shared.names["room_id"].label);
        assert!(shared.never.contains("password"));
    }
}
