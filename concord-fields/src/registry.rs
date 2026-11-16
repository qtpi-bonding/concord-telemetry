use std::collections::{BTreeMap, BTreeSet};

use crate::{parse, FieldDef, ParseError, Parsed, RESERVED_NAMESPACE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldVerdict {
    Ok,
    Unregistered,
    Never,
    /// In the reserved `concord.` namespace.
    ReservedPrefix,
}

#[derive(Clone, Debug)]
pub struct Registry {
    pub names: BTreeMap<String, FieldDef>,
    pub never: BTreeSet<String>,
    /// Wrapper macros declared field-bearing in the local registry.
    pub log_macros: BTreeSet<String>,
}

impl Registry {
    pub fn shared() -> Registry {
        from_parsed(crate::shared())
    }

    pub fn with_local(mut self, source: &str) -> Result<Registry, ParseError> {
        let local = from_parsed(parse(source)?);
        for name in local.names.keys() {
            if local.never.contains(name) {
                return Err(ParseError::NeverAndRegistered { name: name.clone() });
            }
            if self.names.contains_key(name) {
                return Err(ParseError::Shadows { name: name.clone() });
            }
            if self.never.contains(name) {
                return Err(ParseError::NeverName { name: name.clone() });
            }
        }
        for name in &local.never {
            if self.names.contains_key(name) {
                return Err(ParseError::NeverAndRegistered { name: name.clone() });
            }
        }
        self.names.extend(local.names);
        self.never.extend(local.never);
        self.log_macros.extend(local.log_macros);
        Ok(self)
    }

    /// True when `name` is registered with `label = true`.
    pub fn is_label(&self, name: &str) -> bool {
        self.names.get(name).is_some_and(|def| def.label)
    }

    /// True when `name` is registered with `free_text = true`.
    pub fn is_free_text(&self, name: &str) -> bool {
        self.names.get(name).is_some_and(|def| def.free_text)
    }

    /// The registered names with `label = true`, sorted.
    pub fn label_names(&self) -> Vec<String> {
        self.names
            .iter()
            .filter(|(_, def)| def.label)
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub fn check(&self, name: &str) -> FieldVerdict {
        if name.starts_with(RESERVED_NAMESPACE) {
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

/// The label names of the shared registry, sorted.
pub fn label_names() -> Vec<String> {
    Registry::shared().label_names()
}

fn from_parsed(parsed: Parsed) -> Registry {
    Registry {
        names: parsed.names,
        never: parsed.never,
        log_macros: parsed.log_macros,
    }
}

fn reserved(name: &str) -> bool {
    matches!(
        name,
        "event.type"
            | "principal.kind"
            | "principal.id"
            | "principal.verified_by"
            | "principal.via"
    )
}
