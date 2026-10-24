use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct BytesType {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(bytes, tag = "2")]
    payload: ::prost::bytes::Bytes,
}

fn main() {}
