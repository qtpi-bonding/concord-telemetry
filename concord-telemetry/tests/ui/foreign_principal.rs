use concord_telemetry::Event;

mod foreign {
    #[derive(::prost::Message)]
    pub struct Principal {
        #[prost(string, tag = "1")]
        pub id: String,
    }
}

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct ForeignPrincipal {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(message, optional, tag = "2")]
    principal: Option<foreign::Principal>,
}

fn main() {}
