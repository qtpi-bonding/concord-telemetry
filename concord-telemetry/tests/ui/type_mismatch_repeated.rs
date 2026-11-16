use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct Repeated {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(int64, repeated, tag = "2")]
    count: Vec<i64>,
}

fn main() {}
