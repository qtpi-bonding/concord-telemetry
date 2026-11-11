use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
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
    ratchet_skip: bool,
    error_context: usize,
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
            ratchet_skip: skip_registry,
            error_context: 0,
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
        let concord_names = ["error", "error_anyhow", "error_at", "error_at_anyhow"];
        if (path.segments.len() == 1 && self.tracing.contains(&name)
            || path.segments.len() == 2 && full.starts_with("tracing::"))
            && (tracing_names.contains(&name.as_str()) || self.tracing.contains(&name))
        {
            return Some(("tracing".to_string(), name));
        }
        if self.registry.log_macros.contains(&name) {
            return Some(("concord".to_string(), name));
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
            FieldVerdict::ReservedPrefix => self.add(mac, "reserved-label-prefix", &name),
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
                && ["error", "error_anyhow", "error_at", "error_at_anyhow"].contains(&original)
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
    let allowed_concord = ["error", "error_anyhow", "error_at", "error_at_anyhow"];
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
        let old_ratchet = self.ratchet_skip;
        if self.visit_attrs(&item.attrs) {
            self.skip_registry = true;
            self.ratchet_skip = true;
        }
        syn::visit::visit_item_mod(self, item);
        self.skip_registry = old;
        self.ratchet_skip = old_ratchet;
    }

    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        let old = self.skip_registry;
        let old_ratchet = self.ratchet_skip;
        if self.visit_attrs(&item.attrs) {
            self.skip_registry = true;
            self.ratchet_skip = true;
        }
        self.check_instrument(&item.attrs, &item.sig);
        if !self.ratchet_skip {
            if let syn::ReturnType::Type(_, ty) = &item.sig.output {
                if let syn::Type::Path(path) = &**ty {
                    if path.path.segments.last().is_some_and(|s| s.ident == "Result")
                        && path.path.segments.last().and_then(|s| match &s.arguments {
                            syn::PathArguments::AngleBracketed(a) => a.args.first(), _ => None
                        }).and_then(|arg| match arg {
                            syn::GenericArgument::Type(_) => path.path.segments.last().and_then(|s| match &s.arguments {
                                syn::PathArguments::AngleBracketed(a) => a.args.get(1), _ => None
                            }),
                            _ => None,
                        }).is_some_and(|arg| matches!(arg, syn::GenericArgument::Type(syn::Type::Path(p)) if p.path.is_ident("String"))) {
                        self.add_span(item.sig.ident.span(), "ratchet.result_string", "Result error type is String");
                    }
                }
            }
        }
        syn::visit::visit_item_fn(self, item);
        self.skip_registry = old;
        self.ratchet_skip = old_ratchet;
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        let old = self.skip_registry;
        let old_ratchet = self.ratchet_skip;
        if self.visit_attrs(&item.attrs) {
            self.skip_registry = true;
            self.ratchet_skip = true;
        }
        self.check_instrument(&item.attrs, &item.sig);
        syn::visit::visit_impl_item_fn(self, item);
        self.skip_registry = old;
        self.ratchet_skip = old_ratchet;
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        let old = self.skip_registry;
        let old_ratchet = self.ratchet_skip;
        if self.visit_attrs(&item.attrs) {
            self.skip_registry = true;
            self.ratchet_skip = true;
        }
        // A method with no body is not instrumented.
        if item.default.is_some() {
            self.check_instrument(&item.attrs, &item.sig);
        }
        syn::visit::visit_trait_item_fn(self, item);
        self.skip_registry = old;
        self.ratchet_skip = old_ratchet;
    }

    fn visit_item_impl(&mut self, item: &'ast ItemImpl) {
        let old = self.skip_registry;
        let old_ratchet = self.ratchet_skip;
        if self.visit_attrs(&item.attrs) {
            self.skip_registry = true;
            self.ratchet_skip = true;
        }
        syn::visit::visit_item_impl(self, item);
        self.skip_registry = old;
        self.ratchet_skip = old_ratchet;
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
        if !self.ratchet_skip {
            let path = match &*call.func {
                Expr::Path(path) => quote_path(&path.path),
                _ => String::new(),
            };
            if path == "tokio::spawn"
                && call.args.first().is_some_and(|arg| {
                    let text = arg.to_token_stream().to_string();
                    !text.contains("instrument") && !text.contains("in_current_span")
                })
            {
                self.add_span(
                    call.func.span(),
                    "ratchet.untraced_spawn",
                    "tokio::spawn without tracing span",
                );
            } else if path == "tokio::sync::mpsc::channel"
                || path == "tokio::sync::mpsc::unbounded_channel"
            {
                self.add_span(call.func.span(), "ratchet.raw_channel", "raw tokio channel");
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if !self.ratchet_skip && matches!(&local.pat, syn::Pat::Wild(_)) {
            if let Some(init) = &local.init {
                if matches!(&*init.expr, Expr::Call(_) | Expr::MethodCall(_)) {
                    self.add_span(
                        init.expr.span(),
                        "ratchet.dropped_result",
                        "result is discarded",
                    );
                }
            }
        }
        syn::visit::visit_local(self, local);
    }

    fn visit_stmt(&mut self, stmt: &'ast syn::Stmt) {
        if !self.ratchet_skip {
            if let syn::Stmt::Expr(Expr::MethodCall(call), Some(_)) = stmt {
                if call.method == "ok" && call.args.is_empty() {
                    self.add_span(
                        call.method.span(),
                        "ratchet.dropped_result",
                        "result is discarded",
                    );
                }
            }
        }
        syn::visit::visit_stmt(self, stmt);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if !self.ratchet_skip && self.error_context > 0 && call.method == "to_string" {
            self.add_span(
                call.method.span(),
                "ratchet.error_to_string",
                "error converted to string",
            );
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_match(&mut self, expr: &'ast syn::ExprMatch) {
        self.visit_expr(&expr.expr);
        for arm in &expr.arms {
            let is_err = arm.pat.to_token_stream().to_string().starts_with("Err (");
            if is_err {
                self.error_context += 1;
            }
            if let Some((_, guard)) = &arm.guard {
                self.visit_expr(guard);
            }
            self.visit_expr(&arm.body);
            if is_err {
                self.error_context -= 1;
            }
        }
    }

    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        let old = self.error_context;
        if old == 0
            && closure.inputs.len() == 1
            && closure.inputs[0].to_token_stream().to_string() == "e"
        {
            self.error_context = 1;
        }
        syn::visit::visit_expr_closure(self, closure);
        self.error_context = old;
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        if !self.ratchet_skip && self.error_context > 0 && mac.path.is_ident("format") {
            self.add_span(
                mac.span(),
                "ratchet.error_to_string",
                "error formatted as string",
            );
        }
        if is_span_macro(&mac.path) {
            self.check_span_macro(mac);
        }
        if let Some((family, name)) = self.macro_family(&mac.path) {
            inspect_macro(self, mac, &family, &name);
        } else {
            recurse_unknown_macro(self, mac);
        }
        syn::visit::visit_macro(self, mac);
    }
}

impl Scanner<'_> {
    fn add_span(&mut self, span: proc_macro2::Span, rule: &str, detail: &str) {
        self.violations.push(Violation {
            path: self.path.clone(),
            line: span.start().line,
            rule: rule.to_string(),
            detail: detail.to_string(),
        });
    }

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
fn is_span_macro(path: &SynPath) -> bool {
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
struct Recorded {
    name: String,
    debug: bool,
}

/// `name = ?expr` and `?name` record through Debug; `name = %expr`,
/// `name = expr` and `%name` do not.
fn recorded(segment: &TokenStream) -> Option<Recorded> {
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
    fn add_label_debug(&mut self, span: proc_macro2::Span, name: &str, how: &str) {
        self.add_span(span, "label-debug", &format!("{name} ({how})"));
    }

    /// A label key written through `?` in a span macro, and the reserved
    /// prefix as a field name.
    fn check_span_macro(&mut self, mac: &Macro) {
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
    fn check_instrument(&mut self, attrs: &[Attribute], sig: &syn::Signature) {
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
        let span = attr.span();
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
