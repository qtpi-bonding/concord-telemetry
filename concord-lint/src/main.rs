use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use concord_fields::Registry;
use concord_lint::baseline;
use concord_lint::scan::{scan_file, Violation};
use concord_lint::LintError;

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("concord-lint: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<u8, LintError> {
    let mut local = None;
    let mut baseline_path = None;
    let mut write_baseline = false;
    let mut paths = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--local" => {
                index += 1;
                local = Some(
                    args.get(index)
                        .ok_or_else(|| LintError::BadArgs("--local needs a path".into()))?
                        .clone(),
                );
            }
            "--baseline" => {
                index += 1;
                baseline_path = Some(
                    args.get(index)
                        .ok_or_else(|| LintError::BadArgs("--baseline needs a path".into()))?
                        .clone(),
                );
            }
            "--write-baseline" => write_baseline = true,
            value if value.starts_with('-') => {
                return Err(LintError::BadArgs(format!("unknown option {value}")));
            }
            value => paths.push(PathBuf::from(value)),
        }
        index += 1;
    }
    if paths.is_empty() {
        return Err(LintError::BadArgs(
            "usage: concord-lint [--local FILE] [--baseline FILE] [--write-baseline] PATH..."
                .into(),
        ));
    }

    let mut registry = Registry::shared();
    if let Some(path) = local {
        let text = fs::read_to_string(&path).map_err(|source| LintError::Io {
            path: PathBuf::from(&path),
            source,
        })?;
        registry = registry.with_local(&text)?;
    }
    let mut violations = Vec::new();
    for path in paths {
        collect(&path, &registry, &mut violations)?;
    }
    violations.sort();

    if write_baseline {
        let path = baseline_path
            .as_deref()
            .ok_or_else(|| LintError::BadArgs("--write-baseline needs --baseline FILE".into()))?;
        baseline::write(Path::new(path), &violations)?;
        return Ok(0);
    }
    let mut visible = violations.clone();
    let mut shrank = false;
    if let Some(path) = baseline_path {
        let entries = baseline::read(Path::new(&path))?;
        (visible, shrank) = baseline::apply(&violations, &entries);
    }
    for violation in &visible {
        println!("{}", violation.display());
    }
    if shrank {
        eprintln!("note: baseline can shrink; some recorded violations are fixed");
    }
    Ok(if visible.is_empty() { 0 } else { 1 })
}

fn collect(path: &Path, registry: &Registry, output: &mut Vec<Violation>) -> Result<(), LintError> {
    if !path.exists() {
        return Err(LintError::BadArgs(format!(
            "no such path: {}",
            path.display()
        )));
    }
    if path.is_dir() {
        if path.file_name().is_some_and(|name| name == "target") {
            return Ok(());
        }
        let entries = fs::read_dir(path).map_err(|source| LintError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| LintError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            collect(&entry.path(), registry, output)?;
        }
    } else if path.extension().is_some_and(|extension| extension == "rs") {
        let source = fs::read_to_string(path).map_err(|source| LintError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        output.extend(scan_file(path, &source, registry)?);
    }
    Ok(())
}
