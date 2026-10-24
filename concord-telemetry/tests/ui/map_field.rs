use concord_telemetry::Event;
use std::collections::HashMap;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct MapField {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(map = "string, string", tag = "2")]
    values: HashMap<String, String>,
}

fn main() {}
