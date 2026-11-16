use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct RepeatedStr {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(string, repeated, tag = "2")]
    room_id: Vec<String>,
}

fn main() {}
