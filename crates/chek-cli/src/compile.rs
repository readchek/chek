//! Builds every source crate in the current Cargo project and copies the
//! components into a repository folder.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

const TARGET: &str = "wasm32-wasip2";

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    target_directory: PathBuf,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    version: String,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    crate_types: Vec<String>,
}

/// Builds the sources and leaves one `<crate>-<version>.wasm` per source in
/// `out`, replacing what an earlier build left there.
pub fn build(out: &Path) -> Result<(), String> {
    let status = Command::new("cargo")
        .args(["build", "--release", "--target", TARGET])
        .status()
        .map_err(|e| format!("couldn't run cargo: {e}"))?;
    if !status.success() {
        return Err(format!(
            "cargo build failed; if the target is missing, run `rustup target add {TARGET}`"
        ));
    }
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("couldn't run cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let metadata: Metadata = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("couldn't read cargo metadata: {e}"))?;
    let release = metadata.target_directory.join(TARGET).join("release");

    std::fs::create_dir_all(out).map_err(|e| format!("couldn't create {}: {e}", out.display()))?;
    for entry in std::fs::read_dir(out).map_err(|e| e.to_string())?.flatten() {
        let path = entry.path();
        let stale = path.extension().is_some_and(|e| e == "wasm")
            || path.file_name().is_some_and(|n| n == "index.json");
        if stale {
            std::fs::remove_file(&path)
                .map_err(|e| format!("couldn't remove {}: {e}", path.display()))?;
        }
    }

    let mut copied = 0;
    for package in &metadata.packages {
        let Some(library) = package
            .targets
            .iter()
            .find(|t| t.crate_types.iter().any(|c| c == "cdylib"))
        else {
            continue;
        };
        let built = release.join(format!("{}.wasm", library.name.replace('-', "_")));
        // The version in the name keeps a cached older component from being
        // mistaken for a new one.
        let published = out.join(format!("{}-{}.wasm", package.name, package.version));
        std::fs::copy(&built, &published)
            .map_err(|e| format!("couldn't copy {}: {e}", built.display()))?;
        copied += 1;
    }
    if copied == 0 {
        return Err(
            "no crate here builds a cdylib; sources need `crate-type = [\"cdylib\"]`".to_owned(),
        );
    }
    Ok(())
}
