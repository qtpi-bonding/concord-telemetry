use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{parse_macro_input, Data, DeriveInput, Error, Fields, Path, Type};

const REGISTRY: &str = include_str!("../../fields.toml");

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    String,
    Bool,
    I32,
    I64,
    U32,
    U64,
    F32,
    F64,
    Enum,
    Message,
    Bytes,
    Map,
    Oneof,
    Other,
}

fn kind_for_name(name: &str) -> Kind {
    match name {
        "string" => Kind::String,
        "bool" => Kind::Bool,
        "int32" | "sint32" | "sfixed32" => Kind::I32,
        "int64" | "sint64" | "sfixed64" => Kind::I64,
        "uint32" | "fixed32" => Kind::U32,
        "uint64" | "fixed64" => Kind::U64,
        "float" => Kind::F32,
        "double" => Kind::F64,
        "bytes" => Kind::Bytes,
        "message" => Kind::Message,
        "map" | "btree_map" => Kind::Map,
        "oneof" => Kind::Oneof,
        "enumeration" => Kind::Enum,
        _ => Kind::Other,
    }
}

fn prost_kind(field: &syn::Field) -> Option<(Kind, Option<String>)> {
    for attr in &field.attrs {
        if !attr.path().is_ident("prost") {
            continue;
        }
        let mut kind = None;
        let mut enum_name = None;
        let _ = attr.parse_nested_meta(|meta| {
            let name = meta.path.get_ident().map(|v| v.to_string());
            if let Some(name) = name {
                let parsed_kind = kind_for_name(&name);
                kind = Some(if matches!(parsed_kind, Kind::Other) {
                    kind.unwrap_or(Kind::Other)
                } else {
                    parsed_kind
                });
                if name == "enumeration" {
                    if let Ok(value) = meta.value() {
                        if let Ok(value) = value.parse::<syn::LitStr>() {
                            enum_name = Some(value.value());
                        }
                    }
                }
            }
            Ok(())
        });
        return kind.map(|k| (k, enum_name));
    }
    None
}

fn inner_type(ty: &Type, wrapper: &str) -> Option<Type> {
    if let Type::Path(p) = ty {
        let seg = p.path.segments.last()?;
        if seg.ident == wrapper {
            if let syn::PathArguments::AngleBracketed(a) = &seg.arguments {
                if let Some(syn::GenericArgument::Type(t)) = a.args.first() {
                    return Some(t.clone());
                }
            }
        }
    }
    None
}
fn path_segments(ty: &Type) -> Option<Vec<String>> {
    let Type::Path(path) = ty else { return None };
    Some(
        path.path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect(),
    )
}
fn is_optional(ty: &Type) -> bool {
    inner_type(ty, "Option").is_some()
}
fn is_repeated(ty: &Type) -> bool {
    inner_type(ty, "Vec").is_some()
}
fn is_timestamp(ty: &Type, name: &str) -> bool {
    let Some(segments) = path_segments(ty) else {
        return false;
    };
    matches!(
        segments.first().map(String::as_str),
        Some("prost_types" | "pbjson_types" | "identity_crypto")
    ) && segments.last().map(String::as_str) == Some(name)
}
fn is_principal(ty: &Type) -> bool {
    let Some(segments) = path_segments(ty) else {
        return false;
    };
    segments.iter().any(|s| s == "concord_telemetry")
        && segments.last().map(String::as_str) == Some("Principal")
}
fn never_names() -> Vec<String> {
    let Some(section) = REGISTRY.split("[never]").nth(1) else {
        return Vec::new();
    };
    let Some(line) = section
        .lines()
        .find(|l| l.trim_start().starts_with("names ="))
    else {
        return Vec::new();
    };
    line.split('[')
        .nth(1)
        .unwrap_or("")
        .split(']')
        .next()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[proc_macro_derive(Event, attributes(concord_event))]
pub fn event_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.into_compile_error().into(),
    }
}

fn expand(input: &DeriveInput) -> Result<proc_macro2::TokenStream, Error> {
    let name = &input.ident;
    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => {
                return Err(Error::new_spanned(
                    input,
                    "Event requires a struct with named fields",
                ))
            }
        },
        _ => {
            return Err(Error::new_spanned(
                input,
                "Event can only be derived for structs",
            ))
        }
    };
    let package = input
        .attrs
        .iter()
        .find(|a| a.path().is_ident("concord_event"))
        .and_then(|a| {
            let mut out = None;
            let _ = a.parse_nested_meta(|m| {
                if m.path.is_ident("package") {
                    if let Ok(v) = m.value() {
                        if let Ok(s) = v.parse::<syn::LitStr>() {
                            out = Some(s.value());
                        }
                    }
                }
                Ok(())
            });
            out
        })
        .ok_or_else(|| {
            Error::new_spanned(input, "Event requires #[concord_event(package = \"...\")]")
        })?;
    let type_name = format!("{}.{}", package, name);
    let never = never_names();
    let mut source = false;
    let mut emits = Vec::new();
    let mut rebuilds = Vec::new();
    for field in fields {
        let ident = field.ident.as_ref().unwrap();
        let fname = ident.to_string();
        if fname == "source_record_id" {
            source = matches!(prost_kind(field).map(|x| x.0), Some(Kind::String))
                && path_segments(&field.ty)
                    .and_then(|s| s.last().cloned())
                    .as_deref()
                    == Some("String");
        }
        if never.iter().any(|n| n == &fname) {
            return Err(Error::new_spanned(
                field,
                format!("field `{fname}` is forbidden by the never-log rule"),
            ));
        }
        let (kind, enum_name) = prost_kind(field).ok_or_else(|| {
            Error::new_spanned(
                field,
                format!("field `{fname}` has no #[prost(...)] attribute"),
            )
        })?;
        if kind == Kind::Other {
            return Err(Error::new_spanned(field, "unknown protobuf field kind"));
        }
        if matches!(kind, Kind::Bytes | Kind::Map | Kind::Oneof) {
            return Err(Error::new_spanned(
                field,
                format!("bytes/map/oneof content stays in the source record"),
            ));
        }
        let ty = &field.ty;
        let repeated = is_repeated(ty);
        let optional = is_optional(ty);
        let base_ty =
            inner_type(ty, if repeated { "Vec" } else { "Option" }).unwrap_or_else(|| ty.clone());
        if repeated
            && kind == Kind::Message
            && (is_timestamp(&base_ty, "Timestamp") || is_timestamp(&base_ty, "Duration"))
        {
            return Err(Error::new_spanned(
                field,
                "repeated Timestamp/Duration fields are not supported",
            ));
        }
        if kind == Kind::Message
            && !is_timestamp(&base_ty, "Timestamp")
            && !is_timestamp(&base_ty, "Duration")
        {
            if !(optional && is_principal(&base_ty)) {
                return Err(Error::new_spanned(field, format!("field `{fname}` is a nested message; only Principal, Timestamp, and Duration are supported")));
            }
        }
        let is_principal = kind == Kind::Message && optional && is_principal(&base_ty);
        if is_principal {
            emits.extend(principal_emits(ident, &fname));
        } else {
            emits.push(emit_field(
                ident,
                &fname,
                kind,
                enum_name.as_deref(),
                optional,
                repeated,
                &base_ty,
            )?);
        }
        let rebuild = rebuild_field(
            ident,
            &fname,
            kind,
            enum_name.as_deref(),
            optional,
            repeated,
            &base_ty,
        )?;
        rebuilds.push(rebuild);
    }
    if !source {
        return Err(Error::new_spanned(
            input,
            "Event requires source_record_id: String",
        ));
    }
    let type_lit = syn::LitStr::new(&type_name, Span::call_site());
    Ok(quote! {
        impl ::concord_telemetry::Event for #name {
            const TYPE_NAME: &'static str = #type_lit;
            fn emit(&self) {
                ::concord_telemetry::__private::tracing::event!(name: #type_lit, target: ::concord_telemetry::EVENT_TARGET, ::concord_telemetry::__private::tracing::Level::INFO, event.type = #type_lit, #(#emits),*);
            }
            fn from_attributes(attrs: &::concord_telemetry::Attrs) -> ::core::result::Result<Self, ::concord_telemetry::RebuildError> {
                ::core::result::Result::Ok(Self { #(#rebuilds),* })
            }
        }
    })
}

fn emit_field(
    id: &syn::Ident,
    _fname: &str,
    kind: Kind,
    en: Option<&str>,
    optional: bool,
    repeated: bool,
    base: &Type,
) -> Result<proc_macro2::TokenStream, Error> {
    if kind == Kind::Enum {
        let ep: Path = syn::parse_str(en.ok_or_else(|| {
            Error::new(Span::call_site(), "enumeration is missing its enum type")
        })?)
        .map_err(|_| Error::new_spanned(base, "unparsable enumeration path"))?;
        if repeated {
            return Ok(
                quote!(#id = ::concord_telemetry::__private::serde_json::to_string(&self.#id.iter().map(|v| #ep::try_from(*v).map(|e| e.as_str_name()).unwrap_or("UNRECOGNIZED")).collect::<::std::vec::Vec<_>>()).expect("enum JSON")),
            );
        }
        if optional {
            return Ok(
                quote!(#id = self.#id.map(|v| #ep::try_from(v).map(|e| e.as_str_name()).unwrap_or("UNRECOGNIZED"))),
            );
        }
        return Ok(
            quote!(#id = #ep::try_from(self.#id).map(|e| e.as_str_name()).unwrap_or("UNRECOGNIZED")),
        );
    }
    if kind == Kind::Message && (is_timestamp(base, "Timestamp") || is_timestamp(base, "Duration"))
    {
        let proto = if is_timestamp(base, "Timestamp") {
            format_ident!("Timestamp")
        } else {
            format_ident!("Duration")
        };
        return Ok(quote!(#id = self.#id.as_ref().map(|v| {
            let t = ::concord_telemetry::__private::prost_types::#proto { seconds: v.seconds, nanos: v.nanos };
            t.to_string()
        })));
    }
    if repeated {
        if kind == Kind::U64 {
            return Ok(
                quote!(#id = ::concord_telemetry::__private::serde_json::to_string(&self.#id.iter().map(|v| v.to_string()).collect::<::std::vec::Vec<_>>()).expect("event JSON")),
            );
        }
        return Ok(
            quote!(#id = ::concord_telemetry::__private::serde_json::to_string(&self.#id).expect("event JSON")),
        );
    }
    if kind == Kind::U64 {
        if optional {
            return Ok(quote!(#id = self.#id.map(|v| v.to_string())));
        }
        return Ok(quote!(#id = self.#id.to_string()));
    }
    if kind == Kind::U32 {
        if optional {
            return Ok(quote!(#id = self.#id.map(<i64 as ::core::convert::From<u32>>::from)));
        }
        return Ok(quote!(#id = <i64 as ::core::convert::From<u32>>::from(self.#id)));
    }
    if optional {
        return Ok(quote!(#id = self.#id.as_ref()));
    }
    Ok(quote!(#id = self.#id))
}

fn principal_emits(id: &syn::Ident, fname: &str) -> Vec<proc_macro2::TokenStream> {
    let kind = syn::LitStr::new(&format!("{fname}.kind"), Span::call_site());
    let pid = syn::LitStr::new(&format!("{fname}.id"), Span::call_site());
    let verified = syn::LitStr::new(&format!("{fname}.verified_by"), Span::call_site());
    let via = syn::LitStr::new(&format!("{fname}.via"), Span::call_site());
    vec![
        quote!(#kind = self.#id.as_ref().map(|p| ::concord_telemetry::PrincipalKind::try_from(p.kind).map(|v| v.as_str_name()).unwrap_or("UNRECOGNIZED"))),
        quote!(#pid = self.#id.as_ref().map(|p| p.id.as_str())),
        quote!(#verified = self.#id.as_ref().map(|p| {
            let values = p.verified_by.iter()
                .map(|v| ::concord_telemetry::Verification::try_from(*v)
                    .map(|v| v.as_str_name()).unwrap_or("UNRECOGNIZED"))
                .collect::<::std::vec::Vec<_>>();
            ::concord_telemetry::__private::serde_json::to_string(&values).expect("principal JSON")
        })),
        quote!(#via = self.#id.as_ref().and_then(|p| p.via.as_deref())),
    ]
}

fn rebuild_field(
    id: &syn::Ident,
    fname: &str,
    kind: Kind,
    en: Option<&str>,
    optional: bool,
    repeated: bool,
    base: &Type,
) -> Result<proc_macro2::TokenStream, Error> {
    let key = syn::LitStr::new(fname, Span::call_site());
    let get = quote!(attrs.get(#key).ok_or_else(|| ::concord_telemetry::RebuildError::Missing(#key.to_string()))?);
    if kind == Kind::Enum {
        let ep: Path = syn::parse_str(
            en.ok_or_else(|| Error::new_spanned(base, "enumeration is missing its enum type"))?,
        )
        .map_err(|_| Error::new_spanned(base, "unparsable enumeration path"))?;
        if repeated {
            return Ok(
                quote!(#id: ::concord_telemetry::__private::serde_json::from_str::<::std::vec::Vec<::std::string::String>>(match #get { ::concord_telemetry::AttrValue::Str(v) => v, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) }).map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?.into_iter().map(|v: ::std::string::String| #ep::from_str_name(&v).ok_or_else(|| ::concord_telemetry::RebuildError::Parse(#key.to_string())).map(|e| e as i32)).collect::<::core::result::Result<::std::vec::Vec<_>,_>>()?),
            );
        }
        let value = quote!(match #get { ::concord_telemetry::AttrValue::Str(v) => #ep::from_str_name(v).ok_or_else(|| ::concord_telemetry::RebuildError::Parse(#key.to_string()))? as i32, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) });
        if optional {
            return Ok(
                quote!(#id: if attrs.contains_key(#key) { ::core::option::Option::Some(#value) } else { ::core::option::Option::None }),
            );
        }
        return Ok(quote!(#id: #value));
    }
    if kind == Kind::Message && (is_timestamp(base, "Timestamp") || is_timestamp(base, "Duration"))
    {
        let proto = if is_timestamp(base, "Timestamp") {
            quote!(Timestamp)
        } else {
            quote!(Duration)
        };
        let parse = quote!(<::concord_telemetry::__private::prost_types::#proto as ::std::str::FromStr>::from_str(v).map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?);
        let value = quote!({ let t = match #get { ::concord_telemetry::AttrValue::Str(v) => #parse, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) }; #[allow(clippy::needless_update)] #base { seconds: t.seconds, nanos: t.nanos, ..<#base as ::core::default::Default>::default() } });
        return Ok(
            quote!(#id: if attrs.contains_key(#key) { ::core::option::Option::Some(#value) } else { ::core::option::Option::None }),
        );
    }
    if kind == Kind::Message {
        let kind_key = syn::LitStr::new(&format!("{fname}.kind"), Span::call_site());
        let id_key = syn::LitStr::new(&format!("{fname}.id"), Span::call_site());
        let verified_key = syn::LitStr::new(&format!("{fname}.verified_by"), Span::call_site());
        let via_key = syn::LitStr::new(&format!("{fname}.via"), Span::call_site());
        return Ok(quote! {
            #id: if !attrs.contains_key(#kind_key) { ::core::option::Option::None } else { ::core::option::Option::Some(::concord_telemetry::Principal {
                kind: match attrs.get(#kind_key) {
                    ::core::option::Option::Some(::concord_telemetry::AttrValue::Str(v)) => ::concord_telemetry::PrincipalKind::from_str_name(v)
                        .ok_or_else(|| ::concord_telemetry::RebuildError::Parse(#kind_key.to_string()))? as i32,
                    _ => return Err(::concord_telemetry::RebuildError::Type(#kind_key.to_string())),
                },
                id: match attrs.get(#id_key) {
                    ::core::option::Option::Some(::concord_telemetry::AttrValue::Str(v)) => v.clone(),
                    ::core::option::Option::None => ::std::string::String::new(),
                    _ => return Err(::concord_telemetry::RebuildError::Type(#id_key.to_string())),
                },
                verified_by: match attrs.get(#verified_key) {
                    ::core::option::Option::Some(::concord_telemetry::AttrValue::Str(v)) => ::concord_telemetry::__private::serde_json::from_str::<::std::vec::Vec<::std::string::String>>(v)
                        .map_err(|_| ::concord_telemetry::RebuildError::Parse(#verified_key.to_string()))?
                        .into_iter().map(|v| ::concord_telemetry::Verification::from_str_name(&v)
                            .ok_or_else(|| ::concord_telemetry::RebuildError::Parse(#verified_key.to_string()))
                            .map(|v| v as i32)).collect::<::core::result::Result<::std::vec::Vec<_>, _>>()?,
                    ::core::option::Option::None => ::std::vec::Vec::new(),
                    _ => return Err(::concord_telemetry::RebuildError::Type(#verified_key.to_string())),
                },
                via: match attrs.get(#via_key) {
                    ::core::option::Option::Some(::concord_telemetry::AttrValue::Str(v)) => ::core::option::Option::Some(v.clone()),
                    ::core::option::Option::None => ::core::option::Option::None,
                    _ => return Err(::concord_telemetry::RebuildError::Type(#via_key.to_string())),
                },
            }) }
        });
    }
    if repeated {
        if kind == Kind::U64 {
            return Ok(
                quote!(#id: ::concord_telemetry::__private::serde_json::from_str::<::std::vec::Vec<::std::string::String>>(match #get { ::concord_telemetry::AttrValue::Str(v) => v, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) }).map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?.into_iter().map(|v| v.parse::<u64>().map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))).collect::<::core::result::Result<::std::vec::Vec<_>, _>>()?),
            );
        }
        return Ok(
            quote!(#id: ::concord_telemetry::__private::serde_json::from_str(match #get { ::concord_telemetry::AttrValue::Str(v) => v, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) }).map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?),
        );
    }
    let value = scalar_value(kind, &key, get);
    if optional {
        return Ok(
            quote!(#id: if attrs.contains_key(#key) { ::core::option::Option::Some(#value) } else { ::core::option::Option::None }),
        );
    }
    Ok(quote!(#id: #value))
}

fn scalar_value(
    kind: Kind,
    key: &syn::LitStr,
    get: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match kind {
        Kind::String => {
            quote!(match #get { ::concord_telemetry::AttrValue::Str(v) => v.clone(), _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) })
        }
        Kind::Bool => {
            quote!(match #get { ::concord_telemetry::AttrValue::Bool(v) => *v, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) })
        }
        Kind::I32 => quote!(match #get {
            ::concord_telemetry::AttrValue::I64(v) => ::core::convert::TryFrom::try_from(*v).map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?,
            ::concord_telemetry::AttrValue::U64(v) => ::core::convert::TryFrom::try_from(*v).map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?,
            _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())),
        }),
        Kind::I64 => quote!(match #get {
            ::concord_telemetry::AttrValue::I64(v) => *v,
            ::concord_telemetry::AttrValue::U64(v) => ::core::convert::TryFrom::try_from(*v).map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?,
            _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())),
        }),
        Kind::U32 => quote!(match #get {
            ::concord_telemetry::AttrValue::I64(v) => ::core::convert::TryFrom::try_from(*v).map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?,
            ::concord_telemetry::AttrValue::U64(v) => ::core::convert::TryFrom::try_from(*v).map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?,
            _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())),
        }),
        Kind::U64 => {
            quote!(match #get { ::concord_telemetry::AttrValue::Str(v) => v.parse().map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?, ::concord_telemetry::AttrValue::I64(v) => ::core::convert::TryFrom::try_from(*v).map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?, ::concord_telemetry::AttrValue::U64(v) => *v, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) })
        }
        Kind::F32 | Kind::F64 => {
            quote!(match #get { ::concord_telemetry::AttrValue::F64(v) => *v as _, ::concord_telemetry::AttrValue::I64(v) => *v as _, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) })
        }
        _ => quote!(return Err(::concord_telemetry::RebuildError::Type(#key.to_string()))),
    }
}
