use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Attribute, Expr, ExprCall, ItemFn, ItemImpl, ItemMod, Macro, Path as SynPath};

use concord_fields::{FieldVerdict, Registry};

use crate::error::LintError;

mod imports;
mod macros;
mod ratchet;
mod spans;

use macros::{inspect_macro, recurse_unknown_macro};
use spans::is_span_macro;

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

pub fn scan_file(
    path: &Path,
    source: &str,
    registry: &Registry,
) -> Result<Vec<Violation>, LintError> {
    let syntax = syn::parse_file(source).map_err(|error| LintError::Parse {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
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
        let concord_names = [
            "error",
            "error_anyhow",
            "warn",
            "warn_anyhow",
            "error_at",
            "error_at_anyhow",
        ];
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
        let mut collector = imports::ImportCollector {
            tracing: &mut self.tracing,
            concord: &mut self.concord,
        };
        collector.visit_file(file);
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
            self.check_result_string(item);
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
            self.check_spawn_and_channel(call);
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if !self.ratchet_skip {
            self.check_wild_local(local);
        }
        syn::visit::visit_local(self, local);
    }

    fn visit_stmt(&mut self, stmt: &'ast syn::Stmt) {
        if !self.ratchet_skip {
            self.check_discarded_ok(stmt);
        }
        syn::visit::visit_stmt(self, stmt);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if !self.ratchet_skip {
            self.check_error_to_string_call(call);
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
        if !self.ratchet_skip {
            self.check_error_formatted(mac);
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
