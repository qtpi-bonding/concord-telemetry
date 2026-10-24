use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
struct MissingPackage {
    #[prost(string, tag = "1")]
    source_record_id: String,
}

fn main() {}
