//! `concord_telemetry.v1`. Consumers can use this module as the extern path
//! for the hand-written protobuf types.

pub mod v1 {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Principal {
        #[prost(enumeration = "PrincipalKind", tag = "1")]
        pub kind: i32,
        #[prost(string, tag = "2")]
        pub id: ::prost::alloc::string::String,
        #[prost(enumeration = "Verification", repeated, tag = "3")]
        pub verified_by: ::prost::alloc::vec::Vec<i32>,
        #[prost(string, optional, tag = "4")]
        pub via: ::core::option::Option<::prost::alloc::string::String>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum PrincipalKind {
        Unspecified = 0,
        Human = 1,
        Agent = 2,
        Daemon = 3,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum Verification {
        Unspecified = 0,
        GrpcCallerAuth = 1,
        DeliveryAttestation = 2,
        DelegationCert = 3,
        OwnerDocument = 4,
        DaemonSessionCert = 5,
    }
}

pub use v1::{Principal, PrincipalKind, Verification};

impl Principal {
    pub fn new(kind: PrincipalKind, id: impl Into<String>) -> Self {
        Self {
            kind: kind as i32,
            id: id.into(),
            verified_by: Vec::new(),
            via: None,
        }
    }

    pub fn with_verified_by(
        mut self,
        verifications: impl IntoIterator<Item = Verification>,
    ) -> Self {
        self.verified_by = verifications
            .into_iter()
            .map(|verification| verification as i32)
            .collect();
        self
    }

    pub fn with_via(mut self, via: impl Into<String>) -> Self {
        self.via = Some(via.into());
        self
    }

    pub fn verified_by_attr(&self) -> String {
        let names: Vec<&str> = self
            .verified_by
            .iter()
            .filter_map(|value| Verification::try_from(*value).ok())
            .map(|verification| verification.as_str_name())
            .collect();
        serde_json::to_string(&names).expect("a vector of strings is JSON serializable")
    }
}

impl PrincipalKind {
    pub fn as_str_name(&self) -> &'static str {
        match self {
            Self::Unspecified => "PRINCIPAL_KIND_UNSPECIFIED",
            Self::Human => "PRINCIPAL_KIND_HUMAN",
            Self::Agent => "PRINCIPAL_KIND_AGENT",
            Self::Daemon => "PRINCIPAL_KIND_DAEMON",
        }
    }

    pub fn from_str_name(value: &str) -> Option<Self> {
        match value {
            "PRINCIPAL_KIND_UNSPECIFIED" => Some(Self::Unspecified),
            "PRINCIPAL_KIND_HUMAN" => Some(Self::Human),
            "PRINCIPAL_KIND_AGENT" => Some(Self::Agent),
            "PRINCIPAL_KIND_DAEMON" => Some(Self::Daemon),
            _ => None,
        }
    }
}

impl Verification {
    pub fn as_str_name(&self) -> &'static str {
        match self {
            Self::Unspecified => "VERIFICATION_UNSPECIFIED",
            Self::GrpcCallerAuth => "VERIFICATION_GRPC_CALLER_AUTH",
            Self::DeliveryAttestation => "VERIFICATION_DELIVERY_ATTESTATION",
            Self::DelegationCert => "VERIFICATION_DELEGATION_CERT",
            Self::OwnerDocument => "VERIFICATION_OWNER_DOCUMENT",
            Self::DaemonSessionCert => "VERIFICATION_DAEMON_SESSION_CERT",
        }
    }

    pub fn from_str_name(value: &str) -> Option<Self> {
        match value {
            "VERIFICATION_UNSPECIFIED" => Some(Self::Unspecified),
            "VERIFICATION_GRPC_CALLER_AUTH" => Some(Self::GrpcCallerAuth),
            "VERIFICATION_DELIVERY_ATTESTATION" => Some(Self::DeliveryAttestation),
            "VERIFICATION_DELEGATION_CERT" => Some(Self::DelegationCert),
            "VERIFICATION_OWNER_DOCUMENT" => Some(Self::OwnerDocument),
            "VERIFICATION_DAEMON_SESSION_CERT" => Some(Self::DaemonSessionCert),
            _ => None,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrincipalJson {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    kind: String,
    #[serde(default)]
    id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty", alias = "verified_by")]
    verified_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    via: Option<String>,
}

impl serde::Serialize for Principal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let kind = PrincipalKind::try_from(self.kind)
            .map(|kind| {
                if kind == PrincipalKind::Unspecified {
                    String::new()
                } else {
                    kind.as_str_name().to_owned()
                }
            })
            .unwrap_or_default();
        let verified_by = self
            .verified_by
            .iter()
            .filter_map(|value| Verification::try_from(*value).ok())
            .filter(|verification| *verification != Verification::Unspecified)
            .map(|verification| verification.as_str_name().to_owned())
            .collect();
        PrincipalJson {
            kind,
            id: self.id.clone(),
            verified_by,
            via: self.via.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Principal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let json = PrincipalJson::deserialize(deserializer)?;
        let kind = if json.kind.is_empty() {
            PrincipalKind::Unspecified
        } else {
            PrincipalKind::from_str_name(&json.kind)
                .ok_or_else(|| serde::de::Error::custom("unknown PrincipalKind enum name"))?
        };
        let verified_by = json
            .verified_by
            .into_iter()
            .map(|name| {
                Verification::from_str_name(&name)
                    .ok_or_else(|| serde::de::Error::custom("unknown Verification enum name"))
                    .map(|verification| verification as i32)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            kind: kind as i32,
            id: json.id,
            verified_by,
            via: json.via,
        })
    }
}