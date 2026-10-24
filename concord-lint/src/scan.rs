use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};
use syn::parse::Parser;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Attribute, Expr, ExprCall, ItemFn, ItemImpl, ItemMod, Macro, Path as SynPath};

use crate::registry::{FieldVerdict, Registry};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Violation {
    pub path: String,
    pub line: usize,
    pub rule: String,
    pub detail: String,
}

impl Violation {
    pub fn key(&self) -> String {
        format!("{}: {}: {}", self.path, self.rule, self.detail)
    }

    pub fn display(&self) -> String {
        format!(
            "{}:{}: {}: {}",
            self.path, self.line, self.rule, self.detail
        )
    }
}

pub fn scan_file(path: &Path, source: &str, registry: &Registry) -> Result<Vec<Violation>, String> {
    let syntax = syn::parse_file(source).map_err(|error| error.to_string())?;
    let mut visitor = Scanner::new(path, registry);
    visitor.collect_imports(&syntax);
    visitor.visit_file(&syntax);
    Ok(visitor.violations)
}

struct Scanner<'a> {
    path: String,
    registry: &'a Registry,
    violations: Vec<Violation>,
    tracing: Vec<String>,
    concord: Vec<String>,
    skip_registry: bool,
    binary: bool,
}

impl<'a> Scanner<'a> {
    fn new(path: &Path, registry: &'a Registry) -> Scanner<'a> {
        let text = path.to_string_lossy().replace('\\', "/");
        let binary = path.file_name().is_some_and(|name| name == "main.rs")
            || text.contains("/src/bin/")
            || text.starts_with("src/bin/");
        let is_fixture = text.contains("/tests/fixtures/") || text.starts_with("tests/fixtures/");
        let skip_registry = !is_fixture && (text.starts_with("tests/") || text.contains("/tests/"));
        Scanner {
            path: text.clone(),
            registry,
            violations: Vec::new(),
            tracing: Vec::new(),
            concord: Vec::new(),
            skip_registry,
            binary,
        }
    }

    fn add(&mut self, node: &Macro, rule: &str, detail: &str) {
        let line = node.span().start().line;
        self.violations.push(Violation {
            path: self.path.clone(),
            line,
            rule: rule.to_string(),
            detail: detail.to_string(),
        });
    }

    fn macro_family(&self, path: &SynPath) -> Option<(String, String)> {
        let name = path.segments.last()?.ident.to_string();
        let full = quote_path(path);
        let tracing_names = ["trace", "debug", "info", "warn", "error", "event"];
        let concord_names = ["error", "error_anyhow"];
        if (path.segments.len() == 1 && self.tracing.contains(&name)
            || path.segments.len() == 2 && full.starts_with("tracing::"))
            && (tracing_names.contains(&name.as_str()) || self.tracing.contains(&name))
        {
            return Some(("tracing".to_string(), name));
        }
        if (path.segments.len() == 1 && self.concord.contains(&name)
            || path.segments.len() == 2 && full.starts_with("concord_telemetry::"))
            && (concord_names.contains(&name.as_str()) || self.concord.contains(&name))
        {
            return Some(("concord".to_string(), name));
        }
        None
    }

    fn field(&mut self, mac: &Macro, name: String) {
        if self.skip_registry && self.registry.check(&name) == FieldVerdict::Unregistered {
            return;
        }
        match self.registry.check(&name) {
            FieldVerdict::Ok => {}
            FieldVerdict::Unregistered => self.add(mac, "unregistered-field", &name),
            FieldVerdict::Never => self.add(mac, "never-log-field", &name),
        }
    }

    fn visit_attrs(&mut self, attrs: &[Attribute]) -> bool {
        attrs.iter().any(is_test_attribute)
    }

    fn collect_imports(&mut self, file: &syn::File) {
        let mut collector = ImportCollector {
            tracing: &mut self.tracing,
            concord: &mut self.concord,
        };
        collector.visit_file(file);
    }
}

struct ImportCollector<'a> {
    tracing: &'a mut Vec<String>,
    concord: &'a mut Vec<String>,
}

impl<'a, 'b> Visit<'a> for ImportCollector<'b> {
    fn visit_item_use(&mut self, item: &'a syn::ItemUse) {
        collect_use_tree(&item.tree, &mut Vec::new(), self.tracing, self.concord);
    }
}

fn collect_use_tree(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    tracing: &mut Vec<String>,
    concord: &mut Vec<String>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_tree(&path.tree, prefix, tracing, concord);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let mut path = prefix.clone();
            path.push(name.ident.to_string());
            register_import(path, tracing, concord);
        }
        syn::UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            let original_name = rename.ident.to_string();
            path.push(original_name.clone());
            let original = original_name.as_str();
            let alias = rename.rename.to_string();
            if prefix.first().map(String::as_str) == Some("tracing")
                && ["trace", "debug", "info", "warn", "error", "event"].contains(&original)
            {
                tracing.push(alias);
            } else if prefix.first().map(String::as_str) == Some("concord_telemetry")
                && ["error", "error_anyhow"].contains(&original)
            {
                concord.push(alias);
            } else {
                register_import(path, tracing, concord);
            }
        }
        syn::UseTree::Glob(_) => {
            let mut path = prefix.clone();
            path.push("*".to_string());
            register_import(path, tracing, concord);
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree(item, prefix, tracing, concord);
            }
        }
    }
}

fn register_import(path: Vec<String>, tracing: &mut Vec<String>, concord: &mut Vec<String>) {
    let allowed_tracing = ["trace", "debug", "info", "warn", "error", "event"];
    let allowed_concord = ["error", "error_anyhow"];
    if path.first().map(String::as_str) == Some("tracing") {
        if path.last().map(String::as_str) == Some("*") {
            tracing.extend(allowed_tracing.iter().map(|name| (*name).to_string()));
        } else if let Some(name) = path.last() {
            if allowed_tracing.contains(&name.as_str()) {
                tracing.push(name.clone());
            }
        }
    }
    if path.first().map(String::as_str) == Some("concord_telemetry") {
        if let Some(name) = path.last() {
            if allowed_concord.contains(&name.as_str()) {
                concord.push(name.clone());
            }
        }
    }
}

/// `#[test]`, or a `#[cfg(..)]` that names `test` outside a `not(..)`.
fn is_test_attribute(attr: &Attribute) -> bool {
    if attr
        .path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "test")
    {
        return true;
    }
    if !attr.path().is_ident("cfg") {
        return false;
    }
    match &attr.meta {
        syn::Meta::List(list) => mentions_test(list.tokens.clone()),
        _ => false,
    }
}

fn mentions_test(tokens: TokenStream) -> bool {
    let mut after_not = false;
    for token in tokens {
        match token {
            TokenTree::Ident(ident) if ident == "test" => return true,
            TokenTree::Ident(ident) => after_not = ident == "not",
            TokenTree::Group(group) => {
                if !after_not && mentions_test(group.stream()) {
                    return true;
                }
                after_not = false;
            }
            _ => after_not = false,
        }
    }
    false
}

impl<'ast> Visit<'ast> for Scanner<'_> {
    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        let old = self.skip_registry;
        if self.visit_attrs(&item.attrs) {
            self.skip_registry = true;
        }
        syn::visit::visit_item_mod(self, item);
        self.skip_registry = old;
    }

    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        let old = self.skip_registry;
        if self.visit_attrs(&item.attrs) {
            self.skip_registry = true;
        }
        syn::visit::visit_item_fn(self, item);
        self.skip_registry = old;
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        let old = self.skip_registry;
        if self.visit_attrs(&item.attrs) {
            self.skip_registry = true;
        }
        syn::visit::visit_impl_item_fn(self, item);
        self.skip_registry = old;
    }

    fn visit_item_impl(&mut self, item: &'ast ItemImpl) {
        let old = self.skip_registry;
        if self.visit_attrs(&item.attrs) {
            self.skip_registry = true;
        }
        syn::visit::visit_item_impl(self, item);
        self.skip_registry = old;
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        syn::visit::visit_item_use(self, item);
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if self.binary && is_process_exit(&call.func) {
            if let Expr::Path(path) = &*call.func {
                self.add_expr(&path.path, "process-exit", "process::exit");
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        if let Some((family, name)) = self.macro_family(&mac.path) {
            if family == "tracing" && (name == "span" || name.ends_with("_span")) {
                return;
            }
            inspect_macro(self, mac, &family, &name);
        } else {
            recurse_unknown_macro(self, mac);
        }
        syn::visit::visit_macro(self, mac);
    }
}

impl Scanner<'_> {
    fn add_expr(&mut self, path: &SynPath, rule: &str, detail: &str) {
        let line = path.span().start().line;
        self.violations.push(Violation {
            path: self.path.clone(),
            line,
            rule: rule.to_string(),
            detail: detail.to_string(),
        });
    }
}

fn inspect_macro(scanner: &mut Scanner<'_>, mac: &Macro, family: &str, name: &str) {
    let segments = split_top_level(&mac.tokens);
    let mut start = 0;
    if family == "concord" {
        start = 1;
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
    if family == "tracing" && message.is_none() && !segments.is_empty() {
        scanner.add(mac, "message-has-values", "message is not a string literal");
    }
    let _ = name;
}

fn split_top_level(tokens: &TokenStream) -> Vec<TokenStream> {
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

fn field_name(segment: &TokenStream) -> Option<String> {
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
fn recurse_unknown_macro(scanner: &mut Scanner<'_>, mac: &Macro) {
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

fn is_bare_identifier(segment: &TokenStream) -> bool {
    let mut tokens = segment.clone().into_iter();
    matches!(
        (tokens.next(), tokens.next()),
        (Some(TokenTree::Ident(_)), None)
    )
}

fn is_tracing_option(segment: &TokenStream) -> bool {
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
fn quote_path(path: &SynPath) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}
fn is_process_exit(expr: &Expr) -> bool {
    if let Expr::Path(path) = expr {
        let text = quote_path(&path.path);
        text == "std::process::exit" || text == "process::exit"
    } else {
        false
    }
}
