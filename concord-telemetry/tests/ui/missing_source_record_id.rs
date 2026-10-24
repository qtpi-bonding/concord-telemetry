use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct MissingSource {
    #[prost(string, tag = "1")]
    action_id: String,
}

fn main() {}
