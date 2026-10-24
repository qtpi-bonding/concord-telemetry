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
fn path_string(ty: &Type) -> String {
    quote!(#ty).to_string().replace(' ', "")
}
fn is_optional(ty: &Type) -> bool {
    inner_type(ty, "Option").is_some()
}
fn is_repeated(ty: &Type) -> bool {
    inner_type(ty, "Vec").is_some()
}
fn is_timestamp(ty: &Type, name: &str) -> bool {
    let s = path_string(ty);
    (s.contains("prost_types::") || s.contains("pbjson_types::") || s.contains("identity_crypto::"))
        && s.ends_with(name)
}
fn never_names() -> Vec<String> {
    let Some(line) = REGISTRY
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
                && path_string(&field.ty).ends_with("String");
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
        if matches!(kind, Kind::Bytes | Kind::Map | Kind::Oneof) {
            return Err(Error::new_spanned(
                field,
                format!("field `{fname}` uses a forbidden protobuf field kind"),
            ));
        }
        let ty = &field.ty;
        let repeated = is_repeated(ty);
        let optional = is_optional(ty);
        let base_ty =
            inner_type(ty, if repeated { "Vec" } else { "Option" }).unwrap_or_else(|| ty.clone());
        if kind == Kind::Message
            && !is_timestamp(&base_ty, "Timestamp")
            && !is_timestamp(&base_ty, "Duration")
        {
            let p = path_string(&base_ty);
            if !(optional && p.contains("concord_telemetry") && p.ends_with("Principal")) {
                return Err(Error::new_spanned(field, format!("field `{fname}` is a nested message; only Principal, Timestamp, and Duration are supported")));
            }
        }
        let key = syn::LitStr::new(&fname, Span::call_site());
        let is_principal = kind == Kind::Message
            && optional
            && path_string(&base_ty).contains("concord_telemetry")
            && path_string(&base_ty).ends_with("Principal");
        if is_principal {
            emits.extend(principal_emits(ident));
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
            ty,
            kind,
            enum_name.as_deref(),
            optional,
            repeated,
            &base_ty,
        )?;
        rebuilds.push(rebuild);
        let _ = key;
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
            fn from_attributes(attrs: &::concord_telemetry::Attrs) -> Result<Self, ::concord_telemetry::RebuildError> {
                Ok(Self { #(#rebuilds),* })
            }
        }
    })
}

fn emit_field(
    id: &syn::Ident,
    fname: &str,
    kind: Kind,
    en: Option<&str>,
    optional: bool,
    repeated: bool,
    base: &Type,
) -> Result<proc_macro2::TokenStream, Error> {
    let key = syn::LitStr::new(fname, Span::call_site());
    if kind == Kind::Enum {
        let ep: Path = syn::parse_str(en.ok_or_else(|| {
            Error::new(Span::call_site(), "enumeration is missing its enum type")
        })?)
        .unwrap();
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
            let t = ::concord_telemetry::__private::prost_types::#proto {
                seconds: v.seconds,
                nanos: v.nanos,
            };
            t.to_string()
        })));
    }
    if kind == Kind::Message {
        return Ok(quote!(#id = self.#id.as_ref().map(|p| p.kind).unwrap_or_default()));
    }
    if repeated {
        return Ok(
            quote!(#id = ::concord_telemetry::__private::serde_json::to_string(&self.#id).expect("event JSON")),
        );
    }
    if optional {
        return Ok(quote!(#id = self.#id.as_ref()));
    }
    if kind == Kind::U64 {
        return Ok(quote!(#id = self.#id.to_string()));
    }
    if kind == Kind::U32 {
        return Ok(quote!(#id = i64::from(self.#id)));
    }
    let _ = key;
    Ok(quote!(#id = self.#id))
}

fn principal_emits(id: &syn::Ident) -> Vec<proc_macro2::TokenStream> {
    vec![
        quote!("principal.kind" = self.#id.as_ref().map(|p| ::concord_telemetry::PrincipalKind::try_from(p.kind).map(|v| v.as_str_name()).unwrap_or("UNRECOGNIZED"))),
        quote!("principal.id" = self.#id.as_ref().map(|p| p.id.as_str())),
        quote!("principal.verified_by" = self.#id.as_ref().map(|p| {
            let values = p.verified_by.iter()
                .map(|v| ::concord_telemetry::Verification::try_from(*v)
                    .map(|v| v.as_str_name()).unwrap_or("UNRECOGNIZED"))
                .collect::<::std::vec::Vec<_>>();
            ::concord_telemetry::__private::serde_json::to_string(&values).expect("principal JSON")
        })),
        quote!("principal.via" = self.#id.as_ref().and_then(|p| p.via.as_deref())),
    ]
}

fn rebuild_field(
    id: &syn::Ident,
    fname: &str,
    ty: &Type,
    kind: Kind,
    en: Option<&str>,
    optional: bool,
    repeated: bool,
    base: &Type,
) -> Result<proc_macro2::TokenStream, Error> {
    let key = syn::LitStr::new(fname, Span::call_site());
    let get = quote!(attrs.get(#key).ok_or_else(|| ::concord_telemetry::RebuildError::Missing(#key.to_string()))?);
    if kind == Kind::Enum {
        let ep: Path = syn::parse_str(en.unwrap()).unwrap();
        if repeated {
            return Ok(
                quote!(#id: ::concord_telemetry::__private::serde_json::from_str(match #get { ::concord_telemetry::AttrValue::Str(v) => v, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) }).map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?.into_iter().map(|v: String| #ep::from_str_name(&v).ok_or_else(|| ::concord_telemetry::RebuildError::Parse(#key.to_string())).map(|e| e as i32)).collect::<Result<Vec<_>,_>>()?),
            );
        }
        let value = quote!(match #get { ::concord_telemetry::AttrValue::Str(v) => #ep::from_str_name(v).ok_or_else(|| ::concord_telemetry::RebuildError::Parse(#key.to_string()))? as i32, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) });
        if optional {
            return Ok(quote!(#id: if attrs.contains_key(#key) { Some(#value) } else { None }));
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
        let value = quote!(#base { seconds: { let t = match #get { ::concord_telemetry::AttrValue::Str(v) => #parse, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) }; t.seconds }, nanos: { let t = match #get { ::concord_telemetry::AttrValue::Str(v) => #parse, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) }; t.nanos }, ..Default::default() });
        return Ok(quote!(#id: if attrs.contains_key(#key) { Some(#value) } else { None }));
    }
    if kind == Kind::Message {
        let kind_key = syn::LitStr::new(&format!("{fname}.kind"), Span::call_site());
        let id_key = syn::LitStr::new(&format!("{fname}.id"), Span::call_site());
        let verified_key = syn::LitStr::new(&format!("{fname}.verified_by"), Span::call_site());
        let via_key = syn::LitStr::new(&format!("{fname}.via"), Span::call_site());
        return Ok(quote! {
            #id: if !attrs.contains_key(#kind_key) { None } else { Some(::concord_telemetry::Principal {
                kind: match attrs.get(#kind_key) {
                    Some(::concord_telemetry::AttrValue::Str(v)) => ::concord_telemetry::PrincipalKind::from_str_name(v)
                        .ok_or_else(|| ::concord_telemetry::RebuildError::Parse(#kind_key.to_string()))? as i32,
                    _ => return Err(::concord_telemetry::RebuildError::Missing(#kind_key.to_string())),
                },
                id: match attrs.get(#id_key) {
                    Some(::concord_telemetry::AttrValue::Str(v)) => v.clone(),
                    _ => return Err(::concord_telemetry::RebuildError::Missing(#id_key.to_string())),
                },
                verified_by: match attrs.get(#verified_key) {
                    Some(::concord_telemetry::AttrValue::Str(v)) => ::concord_telemetry::__private::serde_json::from_str::<Vec<String>>(v)
                        .map_err(|_| ::concord_telemetry::RebuildError::Parse(#verified_key.to_string()))?
                        .into_iter().map(|v| ::concord_telemetry::Verification::from_str_name(&v)
                            .ok_or_else(|| ::concord_telemetry::RebuildError::Parse(#verified_key.to_string()))
                            .map(|v| v as i32)).collect::<Result<Vec<_>, _>>()?,
                    _ => return Err(::concord_telemetry::RebuildError::Missing(#verified_key.to_string())),
                },
                via: match attrs.get(#via_key) {
                    Some(::concord_telemetry::AttrValue::Str(v)) => Some(v.clone()),
                    None => None,
                    _ => return Err(::concord_telemetry::RebuildError::Type(#via_key.to_string())),
                },
            }) }
        });
    }
    if repeated {
        return Ok(
            quote!(#id: ::concord_telemetry::__private::serde_json::from_str(match #get { ::concord_telemetry::AttrValue::Str(v) => v, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) }).map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?),
        );
    }
    let value = scalar_value(kind, &key, get);
    if optional {
        return Ok(quote!(#id: if attrs.contains_key(#key) { Some(#value) } else { None }));
    }
    let _ = ty;
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
        Kind::I32 | Kind::I64 | Kind::U32 => quote!(match #get {
            ::concord_telemetry::AttrValue::I64(v) => (*v) as _,
            ::concord_telemetry::AttrValue::U64(v) => (*v) as _,
            _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())),
        }),
        Kind::U64 => {
            quote!(match #get { ::concord_telemetry::AttrValue::Str(v) => v.parse().map_err(|_| ::concord_telemetry::RebuildError::Parse(#key.to_string()))?, ::concord_telemetry::AttrValue::I64(v) if *v >= 0 => *v as u64, ::concord_telemetry::AttrValue::U64(v) => *v, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) })
        }
        Kind::F32 | Kind::F64 => {
            quote!(match #get { ::concord_telemetry::AttrValue::F64(v) => *v as _, ::concord_telemetry::AttrValue::I64(v) => *v as _, _ => return Err(::concord_telemetry::RebuildError::Type(#key.to_string())) })
        }
        _ => quote!(return Err(::concord_telemetry::RebuildError::Type(#key.to_string()))),
    }
}
