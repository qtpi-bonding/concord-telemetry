use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct BadSource {
    #[prost(int64, tag = "1")]
    source_record_id: i64,
}

fn main() {}
