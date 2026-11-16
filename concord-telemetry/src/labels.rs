//! Labels: the entries of a W3C `baggage` header, and the helpers that read
//! and write them on a span.
//!
//! The caps are [`MAX_ENTRIES`](crate::context::MAX_ENTRIES) (24) entries,
//! 64 bytes a key, 256 bytes a value (`MAX_VALUE`) and 8192 bytes for the
//! whole header. An entry's properties (`;name=value`) are dropped. Values
//! and keys travel percent-encoded. The whole `concord.` namespace is
//! reserved and never read from or written to a header.
//!
//! [`parse`] and [`format`] are pure and work in every build. [`of`],
//! [`add_once`] and [`record_once`] read and write a span's labels, which
//! exist only with the `otel` feature and an installed exporter: without
//! them they do nothing.

#[cfg(feature = "otel")]
use crate::baggage;
use crate::carrier::context::MAX_ENTRIES;
use tracing::Span;

pub(crate) const MAX_KEY: usize = 64;
/// The most bytes one label value holds.
pub(crate) const MAX_VALUE: usize = 256;
pub(crate) const MAX_HEADER: usize = 8192;

/// The whole `concord.` namespace is reserved: no label or incoming baggage
/// key may start with it, so a key can never collide with an attribute the
/// library itself writes (`concord.label.*`, `concord.label_conflicts`).
pub(crate) const RESERVED_NAMESPACE: &str = "concord.";

/// True when `(key, value)` may be a label: key 1..=64 bytes, not in the
/// reserved namespace, value at most 256 bytes.
pub(crate) fn valid(key: &str, value: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY
        && !key.starts_with(RESERVED_NAMESPACE)
        && value.len() <= MAX_VALUE
}

/// The labels of a `baggage` header, percent-decoded, in order. Properties
/// are dropped, a repeated key keeps its last value, and entries that are
/// malformed, over a cap or in the reserved `concord.` namespace are skipped.
/// A header over 8192 bytes yields nothing. Pure: nothing is recorded.
pub fn parse(header: &str) -> Vec<(String, String)> {
    parse_counted(header).0
}

/// `parse`, and how many entries it dropped.
pub(crate) fn parse_counted(header: &str) -> (Vec<(String, String)>, usize) {
    let segments: Vec<&str> = header.split(',').filter(|s| !s.trim().is_empty()).collect();
    if header.len() > MAX_HEADER {
        return (Vec::new(), segments.len());
    }
    let mut kept: Vec<(String, String)> = Vec::new();
    let mut dropped = 0;
    for segment in segments {
        match parse_entry(segment.trim()) {
            Some((key, value))
                if !key.starts_with(RESERVED_NAMESPACE)
                    && (kept.len() < MAX_ENTRIES || kept.iter().any(|(k, _)| *k == key)) =>
            {
                kept.retain(|(k, _)| *k != key);
                kept.push((key, value));
            }
            _ => dropped += 1,
        }
    }
    (kept, dropped)
}

fn parse_entry(segment: &str) -> Option<(String, String)> {
    let mut parts = segment.split(';');
    let pair = parts.next()?;
    // Properties are not kept, but an empty one makes the entry malformed.
    if parts.any(|property| property.trim().is_empty()) {
        return None;
    }
    let (key, value) = pair.split_once('=')?;
    if value.contains('=') {
        return None;
    }
    let key = decode(key.trim())?;
    let value = decode(value.trim())?;
    valid(&key, &value).then_some((key, value))
}

/// Percent-decodes `text`; a `%` not followed by two hex digits stays as it
/// is. `None` when the result is not UTF-8.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).and_then(|b| hex(*b)),
            bytes.get(i + 2).and_then(|b| hex(*b)),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Percent-encodes everything but printable ASCII other than space, `"`,
/// `,`, `;`, `=` and `%`.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_graphic() && !matches!(byte, b'"' | b',' | b';' | b'=' | b'%') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The entries as a `baggage` header, in order, percent-encoded, and how many
/// were left out: entries past the 24th and any that would take the header
/// over 8192 bytes. The header is `None` when nothing was written.
pub(crate) fn format_counted(entries: &[(String, String)]) -> (Option<String>, usize) {
    let mut header = String::new();
    let mut written = 0;
    for (key, value) in entries.iter().take(MAX_ENTRIES) {
        let piece = format!("{}={}", encode(key), encode(value));
        let extra = piece.len() + usize::from(!header.is_empty());
        if header.len() + extra > MAX_HEADER {
            break;
        }
        if !header.is_empty() {
            header.push(',');
        }
        header.push_str(&piece);
        written += 1;
    }
    (
        (!header.is_empty()).then_some(header),
        entries.len() - written,
    )
}

/// The entries as a `baggage` header, in order, percent-encoded. It stops
/// before the 24-entry or 8192-byte cap is passed; entries left out are not
/// reported. `None` when there is nothing to send. Pure.
pub fn format(entries: &[(String, String)]) -> Option<String> {
    format_counted(entries).0
}

/// The value of label `key` on `span`. `None` when it has none, and without
/// the `otel` feature or an installed exporter.
pub fn of(span: &Span, key: &str) -> Option<String> {
    #[cfg(feature = "otel")]
    {
        baggage::entries(span)
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
    #[cfg(not(feature = "otel"))]
    {
        let _ = (span, key);
        None
    }
}

/// Adds label `key` to `span` unless it already has one or `value` is empty.
/// Over the label caps it is dropped and recorded as
/// `concord.trace.baggage_dropped`. Does nothing without the `otel` feature
/// or an installed exporter.
pub fn add_once(span: &Span, key: &'static str, value: &str) {
    #[cfg(feature = "otel")]
    if !value.is_empty() && of(span, key).is_none() {
        baggage::add(span, key, value.to_owned());
    }
    #[cfg(not(feature = "otel"))]
    let _ = (span, key, value);
}

/// Records `value` into `span`'s declared `field`, unless the span already has
/// a label of that name: the label wins, and the span never holds two values
/// for the key. Does nothing without the `otel` feature or an installed
/// exporter.
pub fn record_once(span: &Span, field: &'static str, value: &str) {
    #[cfg(feature = "otel")]
    if of(span, field).is_none() {
        span.record(field, value);
    }
    #[cfg(not(feature = "otel"))]
    let _ = (span, field, value);
}
