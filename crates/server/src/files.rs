//! Large files (build artifacts, package tarballs, container image layers):
//! content-addressed by SHA-256, streamed in and out, never held whole in
//! memory. On disk under `<data>/files`, or in the objects bucket under
//! `files/` when objects live in S3.

use std::path::{Path, PathBuf};
#[cfg(feature = "s3")]
use std::sync::Arc;

use axum::body::Bytes;
use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{ApiError, ApiResult};
use crate::objects::ObjectsConfig;

/// Bytes streamed back per chunk.
const CHUNK: usize = 256 * 1024;

pub type ByteStream = futures_util::stream::BoxStream<'static, std::io::Result<Bytes>>;

pub struct Files {
    tmp: PathBuf,
    root: Root,
}

enum Root {
    Local(PathBuf),
    #[cfg(feature = "s3")]
    S3(Arc<dyn object_store::ObjectStore>),
}

/// The hex of a `sha256:<hex>` digest (as OCI writes them) or of a bare
/// lowercase hex digest.
#[must_use]
pub fn hex_of(digest: &str) -> Option<&str> {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    (hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(hex)
}

fn io(e: impl std::fmt::Display) -> ApiError {
    ApiError::internal("files", e.to_string())
}

impl Files {
    pub fn open(objects: &ObjectsConfig, data: &Path) -> anyhow::Result<Self> {
        let tmp = data.join("files-tmp");
        std::fs::create_dir_all(&tmp)?;
        let root = match objects {
            ObjectsConfig::Local(_) => {
                let dir = data.join("files");
                std::fs::create_dir_all(&dir)?;
                Root::Local(dir)
            }
            #[cfg(feature = "s3")]
            ObjectsConfig::S3 { bucket } => Root::S3(Arc::new(
                object_store::aws::AmazonS3Builder::from_env()
                    .with_bucket_name(bucket)
                    .build()?,
            )),
        };
        Ok(Self { tmp, root })
    }

    fn local_path(dir: &Path, hex: &str) -> PathBuf {
        dir.join(&hex[..2]).join(&hex[2..])
    }

    #[cfg(feature = "s3")]
    fn key(hex: &str) -> object_store::path::Path {
        object_store::path::Path::from(format!("files/sha256/{}/{}", &hex[..2], &hex[2..]))
    }

    /// A temporary file for an upload that arrives in pieces (container
    /// registry uploads), named by its id.
    #[must_use]
    pub fn upload_path(&self, id: &str) -> Option<PathBuf> {
        (!id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
        .then(|| self.tmp.join(format!("upload-{id}")))
    }

    /// Streams `body` in, up to `max` bytes. Returns the SHA-256 (hex) and
    /// the size.
    pub async fn put_stream<S, E>(&self, body: S, max: u64) -> ApiResult<(String, u64)>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::fmt::Display,
    {
        let mut suffix = [0u8; 12];
        rand::fill(&mut suffix);
        let tmp = self.tmp.join(format!("in-{}", hex::encode(suffix)));
        let result = async {
            let mut file = tokio::fs::File::create(&tmp).await.map_err(io)?;
            append(&mut file, body, max, 0).await?;
            file.sync_all().await.map_err(io)?;
            drop(file);
            self.put_file(&tmp).await
        }
        .await;
        let _ = tokio::fs::remove_file(&tmp).await;
        result
    }

    pub async fn put_bytes(&self, bytes: Vec<u8>) -> ApiResult<(String, u64)> {
        let len = bytes.len() as u64;
        let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from(bytes))]);
        self.put_stream(stream, len).await
    }

    /// Moves a complete file into the store (it is consumed).
    pub async fn put_file(&self, path: &Path) -> ApiResult<(String, u64)> {
        let (hex, size) = sha256_file(path).await?;
        match &self.root {
            Root::Local(dir) => {
                let target = Self::local_path(dir, &hex);
                if tokio::fs::try_exists(&target).await.unwrap_or(false) {
                    let _ = tokio::fs::remove_file(path).await;
                } else {
                    if let Some(parent) = target.parent() {
                        tokio::fs::create_dir_all(parent).await.map_err(io)?;
                    }
                    tokio::fs::rename(path, &target).await.map_err(io)?;
                }
            }
            #[cfg(feature = "s3")]
            Root::S3(s3) => {
                use object_store::ObjectStoreExt;
                let mut upload = s3.put_multipart(&Self::key(&hex)).await.map_err(io)?;
                let mut file = tokio::fs::File::open(path).await.map_err(io)?;
                let mut buf = vec![0u8; 8 * 1024 * 1024];
                loop {
                    let n = read_full(&mut file, &mut buf).await?;
                    if n == 0 {
                        break;
                    }
                    upload
                        .put_part(object_store::PutPayload::from(buf[..n].to_vec()))
                        .await
                        .map_err(io)?;
                }
                upload.complete().await.map_err(io)?;
                let _ = tokio::fs::remove_file(path).await;
            }
        }
        Ok((hex, size))
    }

    /// The size of a stored file, if it is there.
    pub async fn size(&self, hex: &str) -> ApiResult<Option<u64>> {
        match &self.root {
            Root::Local(dir) => match tokio::fs::metadata(Self::local_path(dir, hex)).await {
                Ok(m) => Ok(Some(m.len())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(io(e)),
            },
            #[cfg(feature = "s3")]
            Root::S3(s3) => {
                use object_store::ObjectStoreExt;
                match s3.head(&Self::key(hex)).await {
                    Ok(meta) => Ok(Some(meta.size)),
                    Err(object_store::Error::NotFound { .. }) => Ok(None),
                    Err(e) => Err(io(e)),
                }
            }
        }
    }

    /// The file, streamed, with its size.
    pub async fn get(&self, hex: &str) -> ApiResult<Option<(u64, ByteStream)>> {
        match &self.root {
            Root::Local(dir) => {
                let file = match tokio::fs::File::open(Self::local_path(dir, hex)).await {
                    Ok(f) => f,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(e) => return Err(io(e)),
                };
                let size = file.metadata().await.map_err(io)?.len();
                Ok(Some((size, stream_file(file))))
            }
            #[cfg(feature = "s3")]
            Root::S3(s3) => {
                use futures_util::TryStreamExt;
                use object_store::ObjectStoreExt;
                match s3.get(&Self::key(hex)).await {
                    Ok(result) => {
                        let size = result.meta.size;
                        let stream = result.into_stream().map_err(std::io::Error::other).boxed();
                        Ok(Some((size, stream)))
                    }
                    Err(object_store::Error::NotFound { .. }) => Ok(None),
                    Err(e) => Err(io(e)),
                }
            }
        }
    }

    /// Stored files last written before `before`, as hex digests.
    pub async fn stored_before(&self, before: std::time::SystemTime) -> ApiResult<Vec<String>> {
        let mut out = Vec::new();
        match &self.root {
            Root::Local(dir) => {
                let mut outer = tokio::fs::read_dir(dir).await.map_err(io)?;
                while let Some(prefix) = outer.next_entry().await.map_err(io)? {
                    let head = prefix.file_name().to_string_lossy().into_owned();
                    if head.len() != 2 || !prefix.file_type().await.map_err(io)?.is_dir() {
                        continue;
                    }
                    let mut inner = tokio::fs::read_dir(prefix.path()).await.map_err(io)?;
                    while let Some(file) = inner.next_entry().await.map_err(io)? {
                        let hex = format!("{head}{}", file.file_name().to_string_lossy());
                        let old = file
                            .metadata()
                            .await
                            .and_then(|m| m.modified())
                            .is_ok_and(|t| t < before);
                        if old && hex_of(&hex).is_some() {
                            out.push(hex);
                        }
                    }
                }
            }
            #[cfg(feature = "s3")]
            Root::S3(s3) => {
                use futures_util::TryStreamExt;
                let before_ms = before
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
                let prefix = object_store::path::Path::from("files/sha256");
                let mut listed = s3.list(Some(&prefix));
                while let Some(meta) = listed.try_next().await.map_err(io)? {
                    let hex: String = meta
                        .location
                        .as_ref()
                        .trim_start_matches("files/sha256/")
                        .replace('/', "");
                    if meta.last_modified.timestamp_millis() < before_ms && hex_of(&hex).is_some() {
                        out.push(hex);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Removes a stored file (gone already is fine).
    pub async fn remove(&self, hex: &str) -> ApiResult<()> {
        if hex_of(hex).is_none() {
            return Err(ApiError::bad("not a digest"));
        }
        match &self.root {
            Root::Local(dir) => match tokio::fs::remove_file(Self::local_path(dir, hex)).await {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io(e)),
                _ => Ok(()),
            },
            #[cfg(feature = "s3")]
            Root::S3(s3) => {
                use object_store::ObjectStoreExt;
                match s3.delete(&Self::key(hex)).await {
                    Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
                    Err(e) => Err(io(e)),
                }
            }
        }
    }

    /// Removes uploads that never finished, last written before `before`.
    pub async fn sweep_uploads(&self, before: std::time::SystemTime) -> usize {
        let Ok(mut dir) = tokio::fs::read_dir(&self.tmp).await else {
            return 0;
        };
        let mut removed = 0;
        while let Ok(Some(entry)) = dir.next_entry().await {
            let old = entry
                .metadata()
                .await
                .and_then(|m| m.modified())
                .is_ok_and(|t| t < before);
            if old && tokio::fs::remove_file(entry.path()).await.is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// A whole small file (manifests, metadata) in memory.
    pub async fn read(&self, hex: &str, max: u64) -> ApiResult<Option<Vec<u8>>> {
        let Some((size, mut stream)) = self.get(hex).await? else {
            return Ok(None);
        };
        if size > max {
            return Err(ApiError::too_large(format!("larger than {max} bytes")));
        }
        let mut out = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        while let Some(chunk) = stream.next().await {
            out.extend_from_slice(&chunk.map_err(io)?);
        }
        Ok(Some(out))
    }
}

/// Streams a file from where it is to its end.
#[must_use]
pub fn stream_file(file: tokio::fs::File) -> ByteStream {
    futures_util::stream::try_unfold(file, |mut file| async move {
        let mut buf = vec![0u8; CHUNK];
        let n = file.read(&mut buf).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.truncate(n);
        Ok(Some((Bytes::from(buf), file)))
    })
    .boxed()
}

/// Appends a body to an open file, failing past `max` bytes in all
/// (`already` counts what the file held). Returns the new total.
pub async fn append<S, E>(
    file: &mut tokio::fs::File,
    mut body: S,
    max: u64,
    already: u64,
) -> ApiResult<u64>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    let mut size = already;
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| ApiError::bad(format!("upload interrupted: {e}")))?;
        size += chunk.len() as u64;
        if size > max {
            return Err(ApiError::too_large(format!("larger than {max} bytes")));
        }
        file.write_all(&chunk).await.map_err(io)?;
    }
    Ok(size)
}

#[cfg(feature = "s3")]
async fn read_full(file: &mut tokio::fs::File, buf: &mut [u8]) -> ApiResult<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = file.read(&mut buf[filled..]).await.map_err(io)?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

/// SHA-256 (hex) and size of a file.
pub async fn sha256_file(path: &Path) -> ApiResult<(String, u64)> {
    let mut file = tokio::fs::File::open(path).await.map_err(io)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf).await.map_err(io)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(&buf[..n]);
    }
    Ok((hex::encode(hasher.finalize()), size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn files_round_trip_by_digest() {
        let dir = tempfile::tempdir().unwrap();
        let objects = ObjectsConfig::Local(dir.path().join("objects"));
        let files = Files::open(&objects, dir.path()).unwrap();
        let (hex, size) = files.put_bytes(b"hello".to_vec()).await.unwrap();
        assert_eq!(
            hex,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(size, 5);
        assert_eq!(files.read(&hex, 10).await.unwrap().unwrap(), b"hello");
        assert_eq!(files.size(&hex).await.unwrap(), Some(5));
        assert!(files.read(&"0".repeat(64), 10).await.unwrap().is_none());
        assert_eq!(hex_of(&format!("sha256:{hex}")), Some(hex.as_str()));
        assert_eq!(hex_of("sha256:XYZ"), None);
        assert!(files.upload_path("../x").is_none());
    }

    #[tokio::test]
    async fn stored_files_can_be_listed_by_age_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let files =
            Files::open(&ObjectsConfig::Local(dir.path().join("repos")), dir.path()).unwrap();
        let (hex, _) = files.put_bytes(b"left behind".to_vec()).await.unwrap();
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        assert!(
            files.stored_before(past).await.unwrap().is_empty(),
            "too young to go"
        );
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        assert_eq!(files.stored_before(later).await.unwrap(), vec![hex.clone()]);
        files.remove(&hex).await.unwrap();
        files.remove(&hex).await.unwrap();
        assert_eq!(files.size(&hex).await.unwrap(), None);
        assert!(files.remove("../../etc/passwd").await.is_err());
    }
}
