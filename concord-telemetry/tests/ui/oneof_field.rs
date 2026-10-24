use concord_telemetry::Event;

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct OneofField {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(oneof = "choice", tags = "2, 3")]
    choice: Option<Choice>,
}

#[derive(::prost::Oneof)]
enum Choice {
    #[prost(string, tag = "2")]
    Text(String),
    #[prost(int32, tag = "3")]
    Number(i32),
}

fn main() {}
