use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct TypeMismatch {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(string, tag = "2")]
    count: String,
}

fn main() {}
