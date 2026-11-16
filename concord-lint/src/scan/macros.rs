use proc_macro2::{TokenStream, TokenTree};
use syn::parse::Parser;
use syn::visit::Visit;
use syn::{Expr, Macro};

use super::Scanner;

pub(super) fn inspect_macro(scanner: &mut Scanner<'_>, mac: &Macro, family: &str, name: &str) {
    let segments = split_top_level(&mac.tokens);
    let mut start = 0;
    if family == "concord" {
        // The error; `error_at!` also takes file, line and module first.
        start = if name.starts_with("error_at") { 4 } else { 1 };
    }
    let mut message = None;
    for (index, segment) in segments.iter().enumerate().skip(start) {
        if family == "concord" && is_source(segment) {
            continue;
        }
        if family == "tracing" && is_tracing_option(segment) {
            continue;
        }
        if let Some(field) = field_name(segment) {
            scanner.field(mac, field);
            continue;
        }
        if family == "tracing" && index + 1 < segments.len() && is_bare_identifier(segment) {
            scanner.field(mac, segment.to_string());
            continue;
        }
        if family == "tracing" && message.is_none() {
            message = Some(index);
            let text = segment.to_string();
            if !is_string_literal(&text) || has_interpolation(&text) || index + 1 < segments.len() {
                scanner.add(mac, "message-has-values", "message contains values");
            }
        }
    }
}

pub(super) fn split_top_level(tokens: &TokenStream) -> Vec<TokenStream> {
    let mut result = Vec::new();
    let mut current = TokenStream::new();
    let mut angle_depth = 0usize;
    let mut previous_was_colons = false;
    for token in tokens.clone() {
        let is_comma = matches!(token, TokenTree::Punct(ref punct) if punct.as_char() == ',');
        let punctuation = match &token {
            TokenTree::Punct(punct) => Some(punct.as_char()),
            _ => None,
        };
        if punctuation == Some('<') && previous_was_colons {
            angle_depth += 1;
        }
        if punctuation == Some('>') && angle_depth > 0 {
            angle_depth -= 1;
        }
        if is_comma && angle_depth == 0 {
            result.push(current);
            current = TokenStream::new();
        } else {
            current.extend([token]);
        }
        previous_was_colons = punctuation == Some(':');
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

pub(super) fn field_name(segment: &TokenStream) -> Option<String> {
    let tokens: Vec<TokenTree> = segment.clone().into_iter().collect();
    if let Some(equals_index) = tokens
        .iter()
        .position(|token| matches!(token, TokenTree::Punct(p) if p.as_char() == '='))
    {
        let has_other_equals = tokens.iter().enumerate().any(|(index, token)| {
            index != equals_index && matches!(token, TokenTree::Punct(p) if p.as_char() == '=')
        });
        let lone_equals = matches!(&tokens[equals_index], TokenTree::Punct(p) if p.spacing() == proc_macro2::Spacing::Alone);
        if lone_equals && !has_other_equals {
            let prefix = &tokens[..equals_index];
            if prefix.len() == 1 {
                if let TokenTree::Literal(literal) = &prefix[0] {
                    let text = literal.to_string();
                    if text.starts_with('"') || text.starts_with('r') {
                        return Some(text.trim_matches('"').to_string());
                    }
                }
            }
            if !prefix.is_empty()
                && prefix.iter().enumerate().all(|(index, token)| {
                    (index % 2 == 0 && matches!(token, TokenTree::Ident(_)))
                        || (index % 2 == 1
                            && matches!(token, TokenTree::Punct(p) if p.as_char() == '.'))
                })
            {
                let name = prefix
                    .iter()
                    .filter_map(|token| match token {
                        TokenTree::Ident(ident) => Some(ident.to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(".");
                return Some(name);
            }
        }
    }
    if matches!(tokens.first(), Some(TokenTree::Punct(p)) if p.as_char() == '%' || p.as_char() == '?')
    {
        return tokens.iter().rev().find_map(|token| match token {
            TokenTree::Ident(ident) => Some(ident.to_string()),
            _ => None,
        });
    }
    None
}

/// Looks for tracing calls inside the body of a macro syn does not understand.
/// Bodies that are comma-separated expressions are visited as expressions;
/// any other body (match arms, guards, patterns) is searched token by token
/// for nested `name!(..)` invocations.
pub(super) fn recurse_unknown_macro(scanner: &mut Scanner<'_>, mac: &Macro) {
    let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
    if let Ok(expressions) = parser.parse2(mac.tokens.clone()) {
        for expression in &expressions {
            scanner.visit_expr(expression);
        }
        return;
    }
    let mut nested = Vec::new();
    collect_nested_macros(mac.tokens.clone(), &mut nested);
    for nested_macro in &nested {
        match scanner.macro_family(&nested_macro.path) {
            Some((family, name)) => inspect_macro(scanner, nested_macro, &family, &name),
            None => recurse_unknown_macro(scanner, nested_macro),
        }
    }
}

fn collect_nested_macros(tokens: TokenStream, found: &mut Vec<Macro>) {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    let mut index = 0;
    while index < tokens.len() {
        let is_invocation = index + 2 < tokens.len()
            && matches!(&tokens[index], TokenTree::Ident(_))
            && matches!(&tokens[index + 1], TokenTree::Punct(p) if p.as_char() == '!')
            && matches!(&tokens[index + 2], TokenTree::Group(_));
        if is_invocation {
            let candidate: TokenStream = tokens[index..index + 3].iter().cloned().collect();
            if let Ok(parsed) = syn::parse2::<Macro>(candidate) {
                found.push(parsed);
            }
            index += 3;
            continue;
        }
        if let TokenTree::Group(group) = &tokens[index] {
            collect_nested_macros(group.stream(), found);
        }
        index += 1;
    }
}

pub(super) fn is_bare_identifier(segment: &TokenStream) -> bool {
    let mut tokens = segment.clone().into_iter();
    matches!(
        (tokens.next(), tokens.next()),
        (Some(TokenTree::Ident(_)), None)
    )
}

pub(super) fn is_tracing_option(segment: &TokenStream) -> bool {
    let text = segment.to_string();
    let upper = text.split("::").last().unwrap_or(&text);
    ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"].contains(&upper.trim())
        || text.starts_with("target :")
        || text.starts_with("parent :")
        || text.starts_with("name :")
        || text.starts_with("Level ::")
}

fn is_source(segment: &TokenStream) -> bool {
    segment.to_string().trim_start().starts_with("source =")
}
fn is_string_literal(text: &str) -> bool {
    text.trim_start().starts_with('"') || text.trim_start().starts_with('r')
}
fn has_interpolation(text: &str) -> bool {
    let stripped = text.replace("{{", "").replace("}}", "");
    stripped.contains('{')
}
