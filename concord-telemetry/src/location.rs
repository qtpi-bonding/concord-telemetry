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

use std::borrow::Cow;
use std::cell::RefCell;

#[derive(Clone)]
pub(crate) struct Location {
    pub file: Option<Cow<'static, str>>,
    pub line: Option<u32>,
    pub module: Option<Cow<'static, str>>,
}

type Slot = RefCell<Option<Location>>;

thread_local! {
    static CALLER: Slot = const { RefCell::new(None) };
    static CURRENT: Slot = const { RefCell::new(None) };
}

/// Restores a thread-local to its previous value when dropped, panics included.
struct Restore(&'static std::thread::LocalKey<Slot>, Option<Location>);

impl Drop for Restore {
    fn drop(&mut self) {
        let previous = self.1.take();
        self.0.with(|cell| *cell.borrow_mut() = previous);
    }
}

fn set(key: &'static std::thread::LocalKey<Slot>, location: Location) -> Restore {
    Restore(key, key.with(|cell| cell.replace(Some(location))))
}

/// Runs `body` with `file`, `line` and `module` as the code location of the
/// records it produces, in place of the location tracing would report. `event!`
/// and `error_at!` call it with their call site; a caller that records on behalf
/// of someone else (an error hook in another crate) passes that code's location.
pub fn emit_at(file: &'static str, line: u32, module: &'static str, body: impl FnOnce()) {
    let _restore = set(
        &CALLER,
        Location {
            file: Some(Cow::Borrowed(file)),
            line: Some(line),
            module: Some(Cow::Borrowed(module)),
        },
    );
    body();
}

/// `emit_at` for a location that is not a macro call site (a panic's).
pub(crate) fn emit_at_owned(file: &str, line: u32, body: impl FnOnce()) {
    let _restore = set(
        &CALLER,
        Location {
            file: Some(Cow::Owned(file.to_owned())),
            line: Some(line),
            module: None,
        },
    );
    body();
}

/// Runs `body` (the bridge's `on_event`) with the location of `meta`, or the
/// `event!` call site when there is one, as the current location.
pub(crate) fn with_event_location(meta: &'static tracing::Metadata<'static>, body: impl FnOnce()) {
    let location = CALLER
        .with(|cell| cell.borrow().clone())
        .unwrap_or(Location {
            file: meta.file().map(Cow::Borrowed),
            line: meta.line(),
            module: meta.module_path().map(Cow::Borrowed),
        });
    let _restore = set(&CURRENT, location);
    body();
}

/// The location of the record being emitted on this thread, if any.
pub(crate) fn current() -> Option<Location> {
    CURRENT.with(|cell| cell.borrow().clone())
}
