use quote::ToTokens;
use syn::spanned::Spanned;
use syn::{Expr, ExprCall, ItemFn, Local, Macro, Stmt};

use super::{quote_path, Scanner};

impl Scanner<'_> {
    /// `ratchet.result_string`: a function returning `Result<_, String>`.
    pub(super) fn check_result_string(&mut self, item: &ItemFn) {
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

    /// `ratchet.untraced_spawn` and `ratchet.raw_channel` on a call expression.
    pub(super) fn check_spawn_and_channel(&mut self, call: &ExprCall) {
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

    /// `ratchet.dropped_result` for `let _ = call()`.
    pub(super) fn check_wild_local(&mut self, local: &Local) {
        if matches!(&local.pat, syn::Pat::Wild(_)) {
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
    }

    /// `ratchet.dropped_result` for a statement `expr.ok();`.
    pub(super) fn check_discarded_ok(&mut self, stmt: &Stmt) {
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

    /// `ratchet.error_to_string` for `.to_string()` inside an error context.
    pub(super) fn check_error_to_string_call(&mut self, call: &syn::ExprMethodCall) {
        if self.error_context > 0 && call.method == "to_string" {
            self.add_span(
                call.method.span(),
                "ratchet.error_to_string",
                "error converted to string",
            );
        }
    }

    /// `ratchet.error_to_string` for `format!` inside an error context.
    pub(super) fn check_error_formatted(&mut self, mac: &Macro) {
        if self.error_context > 0 && mac.path.is_ident("format") {
            self.add_span(
                mac.span(),
                "ratchet.error_to_string",
                "error formatted as string",
            );
        }
    }
}
