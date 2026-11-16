use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct U64Int {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(uint64, tag = "2")]
    count: u64,
    #[prost(fixed64, tag = "3")]
    nodes: u64,
}

fn main() {}
