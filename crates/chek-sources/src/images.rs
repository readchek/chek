//! A disk cache of the images the host fetches for extensions, so a page
//! read twice, or by two people, is fetched from its site once. Files are
//! named by a digest of the extension and the request, and the least
//! recently used are deleted once the cache is over its size.

use std::fs::{self, File, FileTimes};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use tracing::warn;

/// Deleting down to this share of the cap leaves room, so the folder isn't
/// scanned again on the next write.
const EVICT_TO_PERCENT: u64 = 90;

pub(crate) struct ImageCache {
    dir: PathBuf,
    max_bytes: u64,
    /// The bytes the folder holds, counted at startup and kept up to date.
    total: Mutex<u64>,
}

impl ImageCache {
    /// Caches up to `max_bytes` in `dir`, or nothing when it's 0.
    pub(crate) fn open(dir: PathBuf, max_bytes: u64) -> Self {
        let total = if max_bytes == 0 {
            0
        } else {
            entries(&dir).iter().map(|e| e.size).sum()
        };
        let cache = Self {
            dir,
            max_bytes,
            total: Mutex::new(total),
        };
        if max_bytes == 0 {
            // Turning the cache off frees what it held.
            let _ = fs::remove_dir_all(&cache.dir);
        } else {
            cache.evict();
        }
        cache
    }

    /// Whether images are kept at all.
    pub(crate) fn enabled(&self) -> bool {
        self.max_bytes > 0
    }

    /// Whether an image is cached, without reading it.
    pub(crate) fn contains(&self, key: &str) -> bool {
        self.enabled() && self.path(key).is_file()
    }

    /// A cached image's content type and bytes.
    pub(crate) fn get(&self, key: &str) -> Option<(Option<String>, Vec<u8>)> {
        if self.max_bytes == 0 {
            return None;
        }
        let path = self.path(key);
        let data = fs::read(&path).ok()?;
        // Reading counts as use, so the image stays ahead of eviction.
        if let Ok(file) = File::options().write(true).open(&path) {
            let _ = file.set_times(FileTimes::new().set_modified(SystemTime::now()));
        }
        let newline = data.iter().position(|&b| b == b'\n')?;
        let content_type = std::str::from_utf8(&data[..newline])
            .ok()
            .filter(|t| !t.is_empty())
            .map(str::to_owned);
        Some((content_type, data[newline + 1..].to_vec()))
    }

    /// Stores an image. Failing to only means the next read fetches it again.
    pub(crate) fn put(&self, key: &str, content_type: Option<&str>, bytes: &[u8]) {
        if self.max_bytes == 0 || bytes.len() as u64 > self.max_bytes / 10 {
            return;
        }
        // A content type with a newline would end the header early.
        let content_type = content_type.filter(|t| !t.contains('\n')).unwrap_or("");
        let path = self.path(key);
        let partial = path.with_extension("partial");
        let written = fs::create_dir_all(&self.dir)
            .and_then(|()| File::create(&partial))
            .and_then(|mut file| {
                file.write_all(content_type.as_bytes())?;
                file.write_all(b"\n")?;
                file.write_all(bytes)
            })
            .and_then(|()| fs::rename(&partial, &path));
        if let Err(error) = written {
            let _ = fs::remove_file(&partial);
            warn!(%error, "couldn't cache an extension image");
            return;
        }
        let size = (content_type.len() + 1 + bytes.len()) as u64;
        let over = {
            let mut total = self.lock();
            *total += size;
            *total > self.max_bytes
        };
        if over {
            self.evict();
        }
    }

    /// Deletes the least recently used images until the cache is back
    /// under its cap.
    fn evict(&self) {
        let mut entries = entries(&self.dir);
        let mut total: u64 = entries.iter().map(|e| e.size).sum();
        let target = self.max_bytes / 100 * EVICT_TO_PERCENT;
        if total > self.max_bytes {
            entries.sort_by_key(|e| e.modified);
            for entry in entries {
                if total <= target {
                    break;
                }
                if fs::remove_file(&entry.path).is_ok() {
                    total -= entry.size;
                }
            }
        }
        *self.lock() = total;
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(key)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, u64> {
        self.total.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct Entry {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
}

/// The cached images in `dir`, leaving out files being written.
fn entries(dir: &Path) -> Vec<Entry> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    read.flatten()
        .filter(|e| e.path().extension().is_none())
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            meta.is_file().then(|| Entry {
                path: e.path(),
                size: meta.len(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            })
        })
        .collect()
}
