use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct U64Str {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(uint64, tag = "2")]
    room_id: u64,
}

fn main() {}
