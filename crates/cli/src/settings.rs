//! Settings shared by every repository of this person (`hb config --global`),
//! in `settings.json` inside `hb`'s configuration directory.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

fn path() -> Option<PathBuf> {
    crate::account::config_dir().map(|dir| dir.join("settings.json"))
}

pub fn all() -> BTreeMap<String, String> {
    path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default()
}

pub fn get(key: &str) -> Option<String> {
    all().remove(key)
}

fn save(settings: &BTreeMap<String, String>) -> Result<()> {
    let path = path().ok_or_else(|| anyhow!("cannot find a configuration directory"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let data = serde_json::to_vec_pretty(settings)?;
    std::fs::write(&path, data).with_context(|| format!("writing {}", path.display()))
}

pub fn set(key: &str, value: &str) -> Result<()> {
    let mut settings = all();
    settings.insert(key.to_owned(), value.to_owned());
    save(&settings)
}

pub fn unset(key: &str) -> Result<bool> {
    let mut settings = all();
    let removed = settings.remove(key).is_some();
    if removed {
        save(&settings)?;
    }
    Ok(removed)
}
