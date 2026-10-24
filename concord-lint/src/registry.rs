use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDef {
    pub ty: String,
    pub meaning: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldVerdict {
    Ok,
    Unregistered,
    Never,
}

#[derive(Clone, Debug)]
pub struct Registry {
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
    #[serde(rename = "type")]
    ty: String,
    meaning: String,
}

#[derive(Default, Deserialize)]
struct Never {
    #[serde(default)]
    names: Vec<String>,
}

impl Registry {
    pub fn shared() -> Registry {
        parse(include_str!("../../fields.toml")).expect("embedded registry is valid")
    }

    pub fn with_local(mut self, source: &str) -> Result<Registry, String> {
        let local = parse(source)?;
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

    pub fn check(&self, name: &str) -> FieldVerdict {
        if self.never.contains(name) {
            FieldVerdict::Never
        } else if self.names.contains_key(name) || reserved(name) {
            FieldVerdict::Ok
        } else {
            FieldVerdict::Unregistered
        }
    }
}

fn parse(source: &str) -> Result<Registry, String> {
    let document: Document = toml::from_str(source).map_err(|error| error.to_string())?;
    let names = document
        .fields
        .into_iter()
        .map(|(name, value)| {
            (
                name,
                FieldDef {
                    ty: value.ty,
                    meaning: value.meaning,
                },
            )
        })
        .collect();
    Ok(Registry {
        names,
        never: document.never.names.into_iter().collect(),
    })
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
