//! Where repository objects live: a local directory or S3-compatible storage,
//! behind an in-memory cache.
//!
//! Objects are immutable and content-addressed, so caching them never needs
//! invalidation, and every replica can share one bucket.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use hebra_core::store::{Backend, FsBackend, Key, Store};

/// Objects larger than this are not cached.
const MAX_CACHED_OBJECT: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub enum ObjectsConfig {
    /// `<data>/repos/<repo>/objects`
    Local(PathBuf),
    /// One bucket; objects under `repos/<repo>/objects/`.
    #[cfg(feature = "s3")]
    S3 { bucket: String },
}

pub struct Objects {
    root: Root,
    keys: Vec<Key>,
    cache: Arc<Cache>,
}

enum Root {
    Local(PathBuf),
    #[cfg(feature = "s3")]
    S3(Arc<dyn object_store::ObjectStore>, tokio::runtime::Handle),
}

impl Objects {
    pub fn open(
        config: &ObjectsConfig,
        keys: Vec<Key>,
        cache_bytes: usize,
    ) -> anyhow::Result<Self> {
        let root = match config {
            ObjectsConfig::Local(dir) => Root::Local(dir.clone()),
            #[cfg(feature = "s3")]
            ObjectsConfig::S3 { bucket } => {
                // Credentials, region and endpoint come from the standard
                // AWS_* variables (or the pod's IAM role / IRSA).
                let s3 = object_store::aws::AmazonS3Builder::from_env()
                    .with_bucket_name(bucket)
                    .build()?;
                Root::S3(Arc::new(s3), tokio::runtime::Handle::current())
            }
        };
        Ok(Self {
            root,
            keys,
            cache: Arc::new(Cache::new(cache_bytes)),
        })
    }

    #[must_use]
    pub fn encrypted(&self) -> bool {
        !self.keys.is_empty()
    }

    /// Creates the place for a new repository's objects.
    pub fn create(&self, repo: &str) -> std::io::Result<()> {
        match &self.root {
            Root::Local(dir) => std::fs::create_dir_all(dir.join(repo).join("objects")),
            #[cfg(feature = "s3")]
            Root::S3(..) => Ok(()),
        }
    }

    /// The object store of one repository. Use from blocking threads only.
    #[must_use]
    pub fn store(&self, repo: &str) -> Store {
        let inner: Arc<dyn Backend> = match &self.root {
            Root::Local(dir) => Arc::new(FsBackend::new(dir.join(repo).join("objects"))),
            #[cfg(feature = "s3")]
            Root::S3(s3, handle) => Arc::new(S3Backend {
                s3: Arc::clone(s3),
                handle: handle.clone(),
                prefix: format!("repos/{repo}/objects"),
            }),
        };
        let backend = Arc::new(Cached {
            inner,
            cache: Arc::clone(&self.cache),
            scope: repo.to_owned(),
        });
        Store::with_backend(backend, self.keys.clone())
    }
}

// ------------------------------------------------------------------ cache

/// A byte-bounded FIFO cache of stored (still encrypted) object files.
struct Cache {
    limit: usize,
    state: Mutex<CacheState>,
}

#[derive(Default)]
struct CacheState {
    bytes: usize,
    map: HashMap<String, Arc<Vec<u8>>>,
    order: VecDeque<String>,
}

impl Cache {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            state: Mutex::new(CacheState::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn get(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        self.lock().map.get(key).cloned()
    }

    fn put(&self, key: String, bytes: Vec<u8>) {
        if bytes.len() > MAX_CACHED_OBJECT || bytes.len() > self.limit {
            return;
        }
        let mut s = self.lock();
        if s.map.contains_key(&key) {
            return;
        }
        s.bytes += bytes.len();
        s.order.push_back(key.clone());
        s.map.insert(key, Arc::new(bytes));
        while s.bytes > self.limit {
            let Some(old) = s.order.pop_front() else {
                break;
            };
            if let Some(v) = s.map.remove(&old) {
                s.bytes -= v.len();
            }
        }
    }
}

#[derive(Debug)]
struct Cached {
    inner: Arc<dyn Backend>,
    cache: Arc<Cache>,
    scope: String,
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cache({} bytes)", self.limit)
    }
}

impl Backend for Cached {
    fn get(&self, key: &str) -> std::io::Result<Option<Vec<u8>>> {
        let scoped = format!("{}/{key}", self.scope);
        if let Some(hit) = self.cache.get(&scoped) {
            return Ok(Some(hit.as_ref().clone()));
        }
        let found = self.inner.get(key)?;
        if let Some(bytes) = &found {
            self.cache.put(scoped, bytes.clone());
        }
        Ok(found)
    }

    fn put(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.inner.put(key, bytes)?;
        self.cache
            .put(format!("{}/{key}", self.scope), bytes.to_vec());
        Ok(())
    }

    fn exists(&self, key: &str) -> std::io::Result<bool> {
        if self.cache.get(&format!("{}/{key}", self.scope)).is_some() {
            return Ok(true);
        }
        self.inner.exists(key)
    }
}

// --------------------------------------------------------------------- s3

#[cfg(feature = "s3")]
struct S3Backend {
    s3: Arc<dyn object_store::ObjectStore>,
    handle: tokio::runtime::Handle,
    prefix: String,
}

#[cfg(feature = "s3")]
impl std::fmt::Debug for S3Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "S3Backend({})", self.prefix)
    }
}

#[cfg(feature = "s3")]
impl S3Backend {
    fn path(&self, key: &str) -> object_store::path::Path {
        object_store::path::Path::from(format!("{}/{key}", self.prefix))
    }
}

#[cfg(feature = "s3")]
impl Backend for S3Backend {
    fn get(&self, key: &str) -> std::io::Result<Option<Vec<u8>>> {
        use object_store::ObjectStoreExt;
        let path = self.path(key);
        self.handle.block_on(async {
            match self.s3.get(&path).await {
                Ok(result) => Ok(Some(
                    result
                        .bytes()
                        .await
                        .map_err(std::io::Error::other)?
                        .to_vec(),
                )),
                Err(object_store::Error::NotFound { .. }) => Ok(None),
                Err(e) => Err(std::io::Error::other(e)),
            }
        })
    }

    fn put(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        use object_store::ObjectStoreExt;
        let path = self.path(key);
        let payload = object_store::PutPayload::from(bytes.to_vec());
        // Same id means same content, so overwriting is harmless.
        self.handle
            .block_on(self.s3.put(&path, payload))
            .map(|_| ())
            .map_err(std::io::Error::other)
    }

    fn exists(&self, key: &str) -> std::io::Result<bool> {
        use object_store::ObjectStoreExt;
        let path = self.path(key);
        match self.handle.block_on(self.s3.head(&path)) {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(std::io::Error::other(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_evicts_oldest_first() {
        let cache = Cache::new(10);
        cache.put("a".into(), vec![0; 4]);
        cache.put("b".into(), vec![0; 4]);
        cache.put("c".into(), vec![0; 4]);
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_some());
    }
}
