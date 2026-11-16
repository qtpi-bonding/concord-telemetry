use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct TypedFields {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(int64, tag = "2")]
    count: i64,
    #[prost(string, tag = "3")]
    reason: String,
    #[prost(bool, tag = "4")]
    ok: bool,
}

fn main() {}
