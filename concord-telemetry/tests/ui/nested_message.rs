use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct Nested {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(message, optional, tag = "2")]
    other: Option<OtherMessage>,
}

#[derive(::prost::Message)]
struct OtherMessage {
    #[prost(string, tag = "1")]
    value: String,
}

fn main() {}
