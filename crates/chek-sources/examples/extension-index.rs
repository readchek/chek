//! Writes the `index.json` for a folder of extension components, reading
//! each component's manifest so the index can't disagree with it.
//!
//! ```sh
//! cargo run -p chek-sources --example extension-index -- <folder> [repository name]
//! ```

use std::path::PathBuf;

use chek_sources::repository::{Index, IndexEntry, sha256_hex};
use chek_sources::{Capability, Host, Options};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(
        args.next()
            .ok_or("usage: extension-index <folder> [name]")?,
    );
    let name = args.next();
    // Reading manifests keeps no state, so it goes in a folder of its own.
    let state = std::env::temp_dir().join(format!("chek-extension-index-{}", std::process::id()));
    let host = Host::new(
        &state,
        &Options {
            image_cache_bytes: 0,
            browser_solver: None,
            browser_executable: None,
            worker_executable: None,
        },
    )?;
    let mut files: Vec<_> = std::fs::read_dir(&dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "wasm"))
        .collect();
    files.sort();
    let mut extensions = Vec::new();
    for path in files {
        let bytes = std::fs::read(&path)?;
        let source = host
            .load(&bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let manifest = source.manifest();
        extensions.push(IndexEntry {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            file: path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("component names must be UTF-8")?
                .to_owned(),
            sha256: sha256_hex(&bytes),
            sites: manifest.sites.clone(),
            languages: manifest.languages.clone(),
            capabilities: manifest
                .capabilities
                .iter()
                .map(|c| match c {
                    Capability::Browser => "browser".to_owned(),
                    Capability::BrowserSession => "browser-session".to_owned(),
                    Capability::ImageTransform => "image-transform".to_owned(),
                })
                .collect(),
        });
        println!("{} {} ({})", manifest.id, manifest.version, path.display());
    }
    let _ = std::fs::remove_dir_all(&state);
    let index = Index { name, extensions };
    std::fs::write(
        dir.join("index.json"),
        serde_json::to_string_pretty(&index)? + "\n",
    )?;
    Ok(())
}
