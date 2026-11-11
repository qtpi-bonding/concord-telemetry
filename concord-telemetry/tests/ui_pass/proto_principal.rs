use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct ProtoPrincipal {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(message, optional, tag = "2")]
    principal: Option<::concord_telemetry_proto::v1::Principal>,
}

fn main() {}
