use proc_macro2::{TokenStream, TokenTree};
use syn::spanned::Spanned;
use syn::{Macro, Path as SynPath};

use super::macros::{field_name, is_bare_identifier, is_tracing_option, split_top_level};
use super::Scanner;

const SPAN_MACROS: [&str; 6] = [
    "span",
    "trace_span",
    "debug_span",
    "info_span",
    "warn_span",
    "error_span",
];

/// `info_span!` and its level siblings, bare or under `tracing::`, and
/// `tracing::span!`.
pub(super) fn is_span_macro(path: &SynPath) -> bool {
    let Some(last) = path.segments.last() else {
        return false;
    };
    let name = last.ident.to_string();
    if !SPAN_MACROS.contains(&name.as_str()) {
        return false;
    }
    match path.segments.len() {
        1 => name != "span",
        2 => path.segments[0].ident == "tracing",
        _ => false,
    }
}

/// How a `fields(..)` or span-macro entry records its value.
pub(super) struct Recorded {
    pub(super) name: String,
    pub(super) debug: bool,
}

/// `name = ?expr` and `?name` record through Debug; `name = %expr`,
/// `name = expr` and `%name` do not.
pub(super) fn recorded(segment: &TokenStream) -> Option<Recorded> {
    let name = field_name(segment)?;
    let tokens: Vec<TokenTree> = segment.clone().into_iter().collect();
    let is = |token: Option<&TokenTree>, c: char| matches!(token, Some(TokenTree::Punct(p)) if p.as_char() == c);
    let debug = if is(tokens.first(), '?') {
        true
    } else if is(tokens.first(), '%') {
        false
    } else {
        tokens
            .iter()
            .position(|token| matches!(token, TokenTree::Punct(p) if p.as_char() == '='))
            .is_some_and(|equals| is(tokens.get(equals + 1), '?'))
    };
    Some(Recorded { name, debug })
}

/// tracing-attributes' `RecordType::TYPES_FOR_VALUE` (expand.rs:446-495): a
/// parameter of one of these types, or a reference to one, is recorded by its
/// `Value` implementation, anything else through Debug.
const TYPES_FOR_VALUE: [&str; 30] = [
    "bool",
    "str",
    "u8",
    "i8",
    "u16",
    "i16",
    "u32",
    "i32",
    "u64",
    "i64",
    "u128",
    "i128",
    "f32",
    "f64",
    "usize",
    "isize",
    "String",
    "NonZeroU8",
    "NonZeroI8",
    "NonZeroU16",
    "NonZeroI16",
    "NonZeroU32",
    "NonZeroI32",
    "NonZeroU64",
    "NonZeroI64",
    "NonZeroU128",
    "NonZeroI128",
    "NonZeroUsize",
    "NonZeroIsize",
    "Wrapping",
];

fn records_by_value(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Path(path) => {
            path.path.segments.last().is_some_and(|segment| {
                TYPES_FOR_VALUE.contains(&segment.ident.to_string().as_str())
            })
        }
        syn::Type::Reference(reference) => records_by_value(&reference.elem),
        _ => false,
    }
}

/// The names an `#[instrument]` parameter pattern binds (tracing-attributes'
/// `param_names`: identifiers, references, tuples, tuple structs and structs;
/// slice patterns bind nothing it records), and whether each is
/// recorded by value (a plain identifier of a value type) or through Debug.
fn param_names(pat: &syn::Pat, by_value: bool, out: &mut Vec<(String, bool)>) {
    match pat {
        syn::Pat::Ident(ident) => out.push((ident.ident.to_string(), by_value)),
        syn::Pat::Reference(reference) => param_names(&reference.pat, by_value, out),
        syn::Pat::Tuple(tuple) => tuple.elems.iter().for_each(|p| param_names(p, false, out)),
        syn::Pat::TupleStruct(tuple) => tuple.elems.iter().for_each(|p| param_names(p, false, out)),
        syn::Pat::Struct(strukt) => strukt
            .fields
            .iter()
            .for_each(|field| param_names(&field.pat, false, out)),
        _ => {}
    }
}

impl Scanner<'_> {
    pub(super) fn add_label_debug(&mut self, span: proc_macro2::Span, name: &str, how: &str) {
        self.add_span(span, "label-debug", &format!("{name} ({how})"));
    }

    /// A label key written through `?` in a span macro, and the reserved
    /// prefix as a field name.
    pub(super) fn check_span_macro(&mut self, mac: &Macro) {
        if self.skip_registry {
            return;
        }
        for segment in split_top_level(&mac.tokens) {
            if is_tracing_option(&segment) {
                continue;
            }
            let Some(field) = recorded(&segment) else {
                continue;
            };
            if field.name.starts_with(concord_fields::RESERVED_NAMESPACE) {
                self.add(mac, "reserved-label-prefix", &field.name);
            } else if field.debug && self.registry.is_label(&field.name) {
                self.add_label_debug(mac.span(), &field.name, "span macro");
            }
        }
    }

    /// `#[instrument]`: a label recorded through `fields(name = ?x)`, or
    /// implicitly as a parameter of a type tracing-attributes records through
    /// Debug, unless skipped or shadowed by `fields(..)`.
    pub(super) fn check_instrument(&mut self, attrs: &[syn::Attribute], sig: &syn::Signature) {
        if self.skip_registry {
            return;
        }
        let Some(attr) = attrs.iter().find(|attr| {
            attr.path()
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "instrument")
        }) else {
            return;
        };
        let span = syn::spanned::Spanned::span(attr);
        let tokens = match &attr.meta {
            syn::Meta::List(list) => list.tokens.clone(),
            _ => TokenStream::new(),
        };
        let mut skip_all = false;
        let mut skips: Vec<String> = Vec::new();
        let mut shadowed: Vec<String> = Vec::new();
        for segment in split_top_level(&tokens) {
            let mut parts = segment.clone().into_iter();
            let Some(TokenTree::Ident(head)) = parts.next() else {
                continue;
            };
            let group = match parts.next() {
                Some(TokenTree::Group(group)) => Some(group.stream()),
                _ => None,
            };
            match (head.to_string().as_str(), group) {
                ("skip_all", None) => skip_all = true,
                ("skip", Some(inner)) => {
                    skips.extend(inner.into_iter().filter_map(|token| match token {
                        TokenTree::Ident(ident) => Some(ident.to_string()),
                        _ => None,
                    }))
                }
                ("fields", Some(inner)) => {
                    for field in split_top_level(&inner) {
                        let name = recorded(&field)
                            .map(|recorded| {
                                if recorded.debug && self.registry.is_label(&recorded.name) {
                                    self.add_label_debug(span, &recorded.name, "instrument fields");
                                }
                                if recorded
                                    .name
                                    .starts_with(concord_fields::RESERVED_NAMESPACE)
                                {
                                    self.add_span(span, "reserved-label-prefix", &recorded.name);
                                }
                                recorded.name
                            })
                            .or_else(|| is_bare_identifier(&field).then(|| field.to_string()));
                        shadowed.extend(name);
                    }
                }
                _ => {}
            }
        }
        if skip_all {
            return;
        }
        for input in &sig.inputs {
            let syn::FnArg::Typed(typed) = input else {
                continue;
            };
            let mut names = Vec::new();
            param_names(&typed.pat, records_by_value(&typed.ty), &mut names);
            for (name, by_value) in names {
                if !by_value
                    && self.registry.is_label(&name)
                    && !skips.contains(&name)
                    && !shadowed.contains(&name)
                {
                    self.add_label_debug(span, &name, "instrument parameter");
                }
            }
        }
    }
}
