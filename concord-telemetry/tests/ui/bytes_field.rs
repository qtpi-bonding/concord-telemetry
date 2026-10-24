use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct BytesField {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(bytes, tag = "2")]
    payload: Vec<u8>,
}

fn main() {}
