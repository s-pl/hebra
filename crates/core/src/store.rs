//! Content-addressed object store.
//!
//! Each object lives in `objects/<2 hex>/<62 hex>`. The file holds
//! `<kind>\0<data>` and its name is the BLAKE3 hash of exactly those bytes,
//! so every read can be verified.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Error, IoContext, Result};
use crate::ids::ObjectId;

pub const BLOB: &str = "blob";
pub const TREE: &str = "tree";
pub const COMMIT: &str = "commit";

#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path_of(&self, id: &ObjectId) -> PathBuf {
        let hex = id.hex();
        self.dir.join(&hex[..2]).join(&hex[2..])
    }

    pub fn write(&self, kind: &str, data: &[u8]) -> Result<ObjectId> {
        let id = ObjectId::hash(kind, data);
        let path = self.path_of(&id);
        if path.exists() {
            return Ok(id);
        }
        let parent = path.parent().unwrap_or(&self.dir);
        fs::create_dir_all(parent).at(parent)?;
        let written = write_atomic(&path, |file| {
            file.write_all(kind.as_bytes())?;
            file.write_all(&[0])?;
            file.write_all(data)
        });
        // Another process may have stored the same object concurrently.
        match written {
            Err(_) if path.exists() => Ok(id),
            other => other.map(|()| id),
        }
    }

    pub fn read(&self, kind: &str, id: &ObjectId) -> Result<Vec<u8>> {
        let path = self.path_of(id);
        let raw = fs::read(&path).at(&path)?;
        if blake3::hash(&raw).as_bytes() != id.as_bytes() {
            return Err(Error::CorruptObject(id.hex()));
        }
        let prefix_len = kind.len() + 1;
        if raw.len() < prefix_len || &raw[..kind.len()] != kind.as_bytes() || raw[kind.len()] != 0 {
            return Err(Error::CorruptObject(id.hex()));
        }
        Ok(raw[prefix_len..].to_vec())
    }

    pub fn write_json<T: Serialize>(&self, kind: &str, value: &T) -> Result<ObjectId> {
        self.write(kind, &serde_json::to_vec(value)?)
    }

    pub fn read_json<T: DeserializeOwned>(&self, kind: &str, id: &ObjectId) -> Result<T> {
        Ok(serde_json::from_slice(&self.read(kind, id)?)?)
    }
}

/// Writes a file atomically: temporary file in the same directory, `fsync`,
/// then rename over the destination.
pub(crate) fn write_atomic(
    path: &Path,
    fill: impl FnOnce(&mut fs::File) -> std::io::Result<()>,
) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| Error::InvalidPath(path.display().to_string()))?;
    let mut suffix = [0u8; 8];
    rand::fill(&mut suffix);
    let tmp = dir.join(format!(".tmp-{}", hex::encode(suffix)));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        fill(&mut file)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.at(path)
}
