use concord_telemetry::Event;

mod concord_telemetry_proto_fake {
    pub mod v1 {
        #[derive(::prost::Message)]
        pub struct Principal {
            #[prost(string, tag = "1")]
            pub id: String,
        }
    }
}

#[derive(::prost::Message, Event)]
#[concord_event(package = "ui.events")]
struct LookalikePrincipal {
    #[prost(string, tag = "1")]
    source_record_id: String,
    #[prost(message, optional, tag = "2")]
    principal: Option<concord_telemetry_proto_fake::v1::Principal>,
}

fn main() {}
