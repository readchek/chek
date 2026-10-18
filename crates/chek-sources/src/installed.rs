//! Installed extensions: component files in the data folder, named by their
//! digest so users who install the same version share one file, and the
//! compiled sources kept in memory once loaded.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::repository::{IndexEntry, sha256_hex};
use crate::{Error, Host, Source};

pub struct Extensions {
    host: Host,
    dir: PathBuf,
    loaded: Mutex<HashMap<String, Arc<Source>>>,
}

impl Extensions {
    /// Keeps component files in `dir`, which is made on the first install.
    pub fn new(dir: PathBuf) -> Result<Self, Error> {
        Ok(Self {
            host: Host::new()?,
            dir,
            loaded: Mutex::default(),
        })
    }

    /// Checks a downloaded component against the index entry it was listed
    /// under and stores it. The manifest inside must name the same id,
    /// version and sites, so a repository can't show users one set of sites
    /// and ship another.
    pub fn install(&self, bytes: &[u8], entry: &IndexEntry) -> Result<Arc<Source>, Error> {
        let digest = sha256_hex(bytes);
        let source = self.host.load(bytes)?;
        let manifest = source.manifest();
        let mut declared = manifest.sites.clone();
        let mut listed = entry.sites.clone();
        declared.sort();
        listed.sort();
        if manifest.id != entry.id || manifest.version != entry.version || declared != listed {
            return Err(Error::Mismatch(format!(
                "the index lists {} {} for {:?}, but the component is {} {} for {:?}",
                entry.id, entry.version, entry.sites, manifest.id, manifest.version, manifest.sites
            )));
        }
        std::fs::create_dir_all(&self.dir).map_err(storage)?;
        let path = self.path(&digest);
        if !path.exists() {
            // Written aside and renamed, so a crash never leaves half a file
            // under the name later loads trust.
            let partial = self.dir.join(format!("{digest}.partial"));
            std::fs::write(&partial, bytes).map_err(storage)?;
            std::fs::rename(&partial, &path).map_err(storage)?;
        }
        let source = Arc::new(source);
        self.cache().insert(digest, Arc::clone(&source));
        Ok(source)
    }

    /// The installed component with this digest, compiled the first time
    /// it's asked for.
    pub fn source(&self, digest: &str) -> Result<Arc<Source>, Error> {
        if let Some(source) = self.cache().get(digest) {
            return Ok(Arc::clone(source));
        }
        if !is_digest(digest) {
            return Err(Error::Storage(format!("`{digest}` isn't a digest")));
        }
        let bytes = std::fs::read(self.path(digest)).map_err(storage)?;
        let source = Arc::new(self.host.load(&bytes)?);
        self.cache().insert(digest.to_owned(), Arc::clone(&source));
        Ok(source)
    }

    /// Deletes a component no install uses any more.
    pub fn remove(&self, digest: &str) -> Result<(), Error> {
        self.cache().remove(digest);
        if !is_digest(digest) {
            return Ok(());
        }
        match std::fs::remove_file(self.path(digest)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(storage(e)),
        }
    }

    fn path(&self, digest: &str) -> PathBuf {
        self.dir.join(format!("{digest}.wasm"))
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Source>>> {
        // The map is only ever inserted into or removed from whole, so a
        // panic elsewhere can't leave it inconsistent.
        self.loaded.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn is_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn storage(error: std::io::Error) -> Error {
    Error::Storage(error.to_string())
}
