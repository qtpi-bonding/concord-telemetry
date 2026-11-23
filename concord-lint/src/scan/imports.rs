use syn::visit::Visit;

pub(super) struct ImportCollector<'a> {
    pub(super) tracing: &'a mut Vec<String>,
    pub(super) concord: &'a mut Vec<String>,
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
                && [
                    "error",
                    "error_anyhow",
                    "warn",
                    "warn_anyhow",
                    "error_at",
                    "error_at_anyhow",
                ]
                .contains(&original)
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
    let allowed_concord = [
        "error",
        "error_anyhow",
        "warn",
        "warn_anyhow",
        "error_at",
        "error_at_anyhow",
    ];
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
