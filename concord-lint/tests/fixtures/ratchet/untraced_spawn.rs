fn bad() { tokio::spawn(fut); }
fn good() { tokio::spawn(fut.instrument(span)); tokio::spawn(fut.in_current_span()); }
