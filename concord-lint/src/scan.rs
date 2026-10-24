use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Attribute, Expr, ExprCall, ItemFn, ItemMod, Macro, Path as SynPath};

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
        let binary =
            path.file_name().is_some_and(|name| name == "main.rs") || text.contains("/src/bin/");
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
        if full.starts_with("tracing::")
            || ["trace", "debug", "info", "warn", "error", "event"].contains(&name.as_str())
                && self.tracing.contains(&name)
        {
            return Some(("tracing".to_string(), name));
        }
        if full.starts_with("concord_telemetry::")
            || ["error", "error_anyhow"].contains(&name.as_str()) && self.concord.contains(&name)
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
}

/// `#[test]`, or a `#[cfg(..)]` that names `test` outside a `not(..)`.
fn is_test_attribute(attr: &Attribute) -> bool {
    if attr.path().is_ident("test") {
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

impl<'a> Visit<'a> for Scanner<'a> {
    fn visit_item_mod(&mut self, item: &'a ItemMod) {
        let old = self.skip_registry;
        if self.visit_attrs(&item.attrs) {
            self.skip_registry = true;
        }
        syn::visit::visit_item_mod(self, item);
        self.skip_registry = old;
    }

    fn visit_item_fn(&mut self, item: &'a ItemFn) {
        let old = self.skip_registry;
        if self.visit_attrs(&item.attrs) {
            self.skip_registry = true;
        }
        syn::visit::visit_item_fn(self, item);
        self.skip_registry = old;
    }

    fn visit_item_use(&mut self, item: &'a syn::ItemUse) {
        let text = item.to_token_stream().to_string();
        if text.contains("tracing") && text.contains('*') {
            self.tracing
                .extend(["trace", "debug", "info", "warn", "error", "event"].map(str::to_string));
        }
        for name in ["trace", "debug", "info", "warn", "error", "event"] {
            if text.contains(name) && text.contains("tracing") {
                self.tracing.push(name.to_string());
            }
        }
        for name in ["error", "error_anyhow"] {
            if text.contains(name) && text.contains("concord_telemetry") {
                self.concord.push(name.to_string());
            }
        }
        syn::visit::visit_item_use(self, item);
    }

    fn visit_expr_call(&mut self, call: &'a ExprCall) {
        if self.binary && is_process_exit(&call.func) {
            if let Expr::Path(path) = &*call.func {
                self.add_expr(&path.path, "process-exit", "process::exit");
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_macro(&mut self, mac: &'a Macro) {
        if let Some((family, name)) = self.macro_family(&mac.path) {
            if family == "tracing" && (name == "span" || name.ends_with("_span")) {
                return;
            }
            inspect_macro(self, mac, &family, &name);
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
        if let Some(field) = field_name(segment, scanner.registry) {
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
    for token in tokens.clone() {
        let is_comma = matches!(token, TokenTree::Punct(ref punct) if punct.as_char() == ',');
        let punctuation = match &token {
            TokenTree::Punct(punct) => Some(punct.as_char()),
            _ => None,
        };
        if punctuation == Some('<') {
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
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

fn field_name(segment: &TokenStream, registry: &Registry) -> Option<String> {
    let text = segment.to_string();
    if let Some((name, _)) = text.split_once('=') {
        let name = normalize_field_name(name);
        return Some(name);
    }
    if text.starts_with('%') || text.starts_with('?') {
        let name = text[1..].trim().to_string();
        if registry.check(&name) == FieldVerdict::Unregistered {
            return None;
        }
        return Some(name);
    }
    if text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && registry.check(&text) != FieldVerdict::Unregistered
    {
        return Some(text);
    }
    None
}

fn normalize_field_name(name: &str) -> String {
    name.trim()
        .trim_matches('"')
        .split_whitespace()
        .collect::<String>()
        .replace('.', ".")
}

fn is_bare_identifier(segment: &TokenStream) -> bool {
    let text = segment.to_string();
    !text.is_empty()
        && text
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn is_tracing_option(segment: &TokenStream) -> bool {
    let text = segment.to_string();
    text.starts_with("target :")
        || text.starts_with("parent :")
        || text.starts_with("name :")
        || text.starts_with("Level ::")
}

fn is_source(segment: &TokenStream) -> bool {
    segment.to_string().trim_start().starts_with("source =")
}
fn is_string_literal(text: &str) -> bool {
    text.trim_start().starts_with('"')
}
fn has_interpolation(text: &str) -> bool {
    text.contains('{') && !text.contains("{{")
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
