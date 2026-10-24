use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct NeverLog {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(string, tag = "2")]
    stdout: String,
}

fn main() {}
