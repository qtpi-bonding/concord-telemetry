use concord_telemetry::Event;

#[derive(Clone, PartialEq, ::prost::Message, Event)]
#[concord_event(package = "test.events.v1")]
pub struct ActionStatusChanged {
    #[prost(string, tag = "1")]
    pub source_record_id: String,
    #[prost(string, tag = "2")]
    pub action_id: String,
    #[prost(enumeration = "Status", tag = "3")]
    pub old_status: i32,
    #[prost(enumeration = "Status", tag = "4")]
    pub new_status: i32,
    #[prost(message, optional, tag = "5")]
    pub principal: Option<concord_telemetry::proto::v1::Principal>,
    #[prost(message, optional, tag = "6")]
    pub at: Option<::prost_types::Timestamp>,
    #[prost(message, optional, tag = "7")]
    pub took: Option<::prost_types::Duration>,
    #[prost(int64, tag = "8")]
    pub attempt: i64,
    #[prost(bool, tag = "9")]
    pub retried: bool,
    #[prost(string, repeated, tag = "10")]
    pub labels: Vec<String>,
    #[prost(string, optional, tag = "11")]
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum Status {
    Unspecified = 0,
    Ready = 1,
    Running = 2,
}

impl Status {
    pub fn as_str_name(&self) -> &'static str {
        match self {
            Self::Unspecified => "STATUS_UNSPECIFIED",
            Self::Ready => "READY",
            Self::Running => "RUNNING",
        }
    }

    pub fn from_str_name(value: &str) -> Option<Self> {
        match value {
            "STATUS_UNSPECIFIED" => Some(Self::Unspecified),
            "READY" => Some(Self::Ready),
            "RUNNING" => Some(Self::Running),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, ::prost::Message, Event)]
#[concord_event(package = "test.events.v1")]
pub struct Scalars {
    #[prost(string, tag = "1")]
    pub source_record_id: String,
    #[prost(uint64, tag = "2")]
    pub big: u64,
    #[prost(uint32, tag = "3")]
    pub count: u32,
    #[prost(int64, tag = "4")]
    pub signed: i64,
    #[prost(float, tag = "5")]
    pub ratio: f32,
    #[prost(double, tag = "6")]
    pub precise: f64,
    #[prost(bool, tag = "7")]
    pub enabled: bool,
    #[prost(enumeration = "Status", optional, tag = "8")]
    pub status: Option<i32>,
    #[prost(uint64, optional, tag = "9")]
    pub maybe_big: Option<u64>,
    #[prost(uint32, optional, tag = "10")]
    pub maybe_count: Option<u32>,
    #[prost(string, optional, tag = "11")]
    pub maybe_text: Option<String>,
    #[prost(enumeration = "Status", repeated, tag = "12")]
    pub statuses: Vec<i32>,
    #[prost(uint64, repeated, tag = "13")]
    pub big_values: Vec<u64>,
}

#[derive(Clone, PartialEq, ::prost::Message, Event)]
#[concord_event(package = "test.events.v1")]
pub struct PbjsonStamped {
    #[prost(string, tag = "1")]
    pub source_record_id: String,
    #[prost(message, optional, tag = "2")]
    pub at: Option<::pbjson_types::Timestamp>,
    #[prost(message, optional, tag = "3")]
    pub took: Option<::pbjson_types::Duration>,
}

pub mod identity_crypto {
    #[derive(Clone, Copy, PartialEq, ::prost::Message)]
    pub struct Timestamp {
        #[prost(int64, tag = "1")]
        pub seconds: i64,
        #[prost(int32, tag = "2")]
        pub nanos: i32,
    }
    #[derive(Clone, Copy, PartialEq, ::prost::Message)]
    pub struct Duration {
        #[prost(int64, tag = "1")]
        pub seconds: i64,
        #[prost(int32, tag = "2")]
        pub nanos: i32,
    }
}

#[derive(Clone, PartialEq, ::prost::Message, Event)]
#[concord_event(package = "test.events.v1")]
pub struct IdentityStamped {
    #[prost(string, tag = "1")]
    pub source_record_id: String,
    #[prost(message, optional, tag = "2")]
    pub at: Option<identity_crypto::Timestamp>,
    #[prost(message, optional, tag = "3")]
    pub took: Option<identity_crypto::Duration>,
}
