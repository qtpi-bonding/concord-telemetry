fn bad() { tokio::sync::mpsc::channel(8); tokio::sync::mpsc::unbounded_channel(); }
fn good() { concord_telemetry::channel(8); }
