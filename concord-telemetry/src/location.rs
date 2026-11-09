//! Code location on exported log records.
//!
//! Spans carry `code.file.path`, `code.line.number` and `code.module.name`
//! (tracing-opentelemetry). The OpenTelemetry log bridge can emit location only
//! under its `experimental_metadata_attributes` feature, with older names
//! (`code.filepath`, `code.lineno`, `code.namespace`) plus a `code.filename`,
//! and an `SdkLogRecord` cannot drop attributes afterwards. So the bridge's
//! metadata is left off and `BaggageBridge` publishes the event's location
//! here for the duration of the bridge call; `BaggageLogProcessor` runs on the
//! same thread and adds the span keys from it.
//!
//! `event!` cannot be located by tracing: the derive emits from inside the
//! event type's `emit`, so the event's own metadata points at the type's
//! definition. The `event!` macro therefore records its call site, which wins
//! over the metadata.

use std::cell::Cell;

#[derive(Clone, Copy)]
pub(crate) struct Location {
    pub file: Option<&'static str>,
    pub line: Option<u32>,
    pub module: Option<&'static str>,
}

thread_local! {
    static CALLER: Cell<Option<Location>> = const { Cell::new(None) };
    static CURRENT: Cell<Option<Location>> = const { Cell::new(None) };
}

/// Restores a thread-local to its previous value when dropped, panics included.
struct Restore(
    &'static std::thread::LocalKey<Cell<Option<Location>>>,
    Option<Location>,
);

impl Drop for Restore {
    fn drop(&mut self) {
        self.0.with(|cell| cell.set(self.1));
    }
}

/// Runs `body` (an event's `emit`) with the macro call site as the location of
/// the records it produces.
#[doc(hidden)]
pub fn emit_at(file: &'static str, line: u32, module: &'static str, body: impl FnOnce()) {
    let _restore = Restore(
        &CALLER,
        CALLER.with(|cell| {
            cell.replace(Some(Location {
                file: Some(file),
                line: Some(line),
                module: Some(module),
            }))
        }),
    );
    body();
}

/// Runs `body` (the bridge's `on_event`) with the location of `meta`, or the
/// `event!` call site when there is one, as the current location.
pub(crate) fn with_event_location(meta: &'static tracing::Metadata<'static>, body: impl FnOnce()) {
    let location = CALLER.with(Cell::get).unwrap_or(Location {
        file: meta.file(),
        line: meta.line(),
        module: meta.module_path(),
    });
    let _restore = Restore(&CURRENT, CURRENT.with(|cell| cell.replace(Some(location))));
    body();
}

/// The location of the record being emitted on this thread, if any.
pub(crate) fn current() -> Option<Location> {
    CURRENT.with(Cell::get)
}
