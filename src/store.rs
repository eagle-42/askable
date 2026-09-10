//! Reading and writing records, and the judge's settings.
//!
//! The thin layer that touches the disk, kept apart from what decides.

use crate::config::Config;
use crate::config::Judge;
use crate::record::Record;
use std::path::Path;

pub fn write_record(record: &Record, path: &Path) -> Result<(), String> {
    // Records are measurements of a moment. Two runs in the same second would
    // otherwise leave one of them silently gone.
    if path.exists() {
        return Err(format!(
            "{} already exists; pass --out to write somewhere else",
            path.display()
        ));
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let json =
        serde_json::to_string_pretty(record).map_err(|e| format!("cannot serialise: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

pub fn read_record(path: &Path) -> Result<Record, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("{} is not a record: {e}", path.display()))
}

/// How strict the judge is, from the config file, or the defaults when there is
/// none. Shared by `run --judge` and `judge`, so the two cannot drift apart.
pub fn judge_settings(config: &Path) -> Result<Judge, String> {
    if config.exists() {
        Ok(Config::load(config)?.judge)
    } else {
        Ok(Judge::default())
    }
}
