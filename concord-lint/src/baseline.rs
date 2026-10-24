use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::scan::Violation;

pub fn read(path: &Path) -> Result<BTreeMap<String, usize>, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut result = BTreeMap::new();
    for line in text.lines() {
        let (count, key) = line
            .split_once('\t')
            .ok_or_else(|| "invalid baseline line".to_string())?;
        let count: usize = count
            .parse()
            .map_err(|_| "invalid baseline count".to_string())?;
        result.insert(key.to_string(), count);
    }
    Ok(result)
}

pub fn write(path: &Path, violations: &[Violation]) -> Result<(), String> {
    let mut counts = BTreeMap::new();
    for violation in violations {
        *counts.entry(violation.key()).or_insert(0) += 1;
    }
    let mut text = String::new();
    for (key, count) in counts {
        text.push_str(&format!("{count}\t{key}\n"));
    }
    fs::write(path, text).map_err(|error| error.to_string())
}

pub fn apply(
    violations: &[Violation],
    baseline: &BTreeMap<String, usize>,
) -> (Vec<Violation>, bool) {
    let mut used = BTreeMap::new();
    let mut fresh = Vec::new();
    for violation in violations {
        let entry = used.entry(violation.key()).or_insert(0);
        *entry += 1;
        if *entry > baseline.get(&violation.key()).copied().unwrap_or(0) {
            fresh.push(violation.clone());
        }
    }
    let shrank = baseline
        .iter()
        .any(|(key, count)| used.get(key).copied().unwrap_or(0) < *count);
    (fresh, shrank)
}
