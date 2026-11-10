use std::collections::{BTreeMap, BTreeSet};

pub use concord_fields::FieldDef;
use concord_fields::LABEL_PREFIX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldVerdict {
    Ok,
    Unregistered,
    Never,
    /// Named with the reserved `concord.label.` prefix.
    ReservedPrefix,
}

#[derive(Clone, Debug)]
pub struct Registry {
    pub names: BTreeMap<String, FieldDef>,
    pub never: BTreeSet<String>,
}

impl Registry {
    pub fn shared() -> Registry {
        from_parsed(concord_fields::shared())
    }

    pub fn with_local(mut self, source: &str) -> Result<Registry, String> {
        let local = from_parsed(concord_fields::parse(source)?);
        for name in local.names.keys() {
            if local.never.contains(name) {
                return Err(format!("{name} is registered and never-log"));
            }
            if self.names.contains_key(name) {
                return Err(format!("{name} shadows a shared name"));
            }
            if self.never.contains(name) {
                return Err(format!("{name} is a never-log name"));
            }
        }
        for name in &local.never {
            if self.names.contains_key(name) {
                return Err(format!("{name} is registered and never-log"));
            }
        }
        self.names.extend(local.names);
        self.never.extend(local.never);
        Ok(self)
    }

    /// True when `name` is registered with `label = true`.
    pub fn is_label(&self, name: &str) -> bool {
        self.names.get(name).is_some_and(|def| def.label)
    }

    pub fn check(&self, name: &str) -> FieldVerdict {
        if name.starts_with(LABEL_PREFIX) {
            FieldVerdict::ReservedPrefix
        } else if self.never.contains(name) {
            FieldVerdict::Never
        } else if self.names.contains_key(name) || reserved(name) {
            FieldVerdict::Ok
        } else {
            FieldVerdict::Unregistered
        }
    }
}

fn from_parsed(parsed: concord_fields::Parsed) -> Registry {
    Registry {
        names: parsed.names,
        never: parsed.never,
    }
}

fn reserved(name: &str) -> bool {
    matches!(
        name,
        "event.type"
            | "error.code"
            | "error.class"
            | "error.fingerprint"
            | "error.source"
            | "principal.kind"
            | "principal.id"
            | "principal.verified_by"
            | "principal.via"
    )
}
