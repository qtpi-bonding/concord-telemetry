use proc_macro::TokenStream;

#[proc_macro_derive(Event, attributes(concord_event))]
pub fn event_stub(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}
