//! Writes a repository's `index.json` from the components in its folder,
//! reading each one's own manifest so the index can't disagree with what
//! it ships.

use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};
use wasmtime::component::{Component, Linker, ResourceTable, Val};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

/// Version 1 of `docs/extension-repositories.md`.
#[derive(Debug, Serialize)]
pub struct Index {
    pub spec: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub extensions: Vec<Entry>,
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub file: String,
    pub sha256: String,
    pub sites: Vec<String>,
    pub languages: Vec<String>,
    pub capabilities: Vec<String>,
}

/// Writes `index.json` in `dir` for every `.wasm` file there.
pub fn write(dir: &Path, name: Option<&str>) -> Result<(), String> {
    let index = build(dir, name)?;
    let json = serde_json::to_string_pretty(&index).map_err(|e| e.to_string())? + "\n";
    let path = dir.join("index.json");
    std::fs::write(&path, json).map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    println!(
        "Wrote {} listing {} extension(s).",
        path.display(),
        index.extensions.len()
    );
    Ok(())
}

pub fn build(dir: &Path, name: Option<&str>) -> Result<Index, String> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("couldn't read {}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "wasm"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("{} has no .wasm components", dir.display()));
    }
    let reader = ManifestReader::new()?;
    let mut extensions = Vec::new();
    for path in files {
        let bytes =
            std::fs::read(&path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
        let mut entry = reader
            .read(&bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        entry.file = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("component names must be UTF-8")?
            .to_owned();
        entry.sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        println!("{} {} ({})", entry.id, entry.version, entry.file);
        extensions.push(entry);
    }
    Ok(Index {
        spec: 1,
        name: name.map(str::to_owned),
        extensions,
    })
}

/// Instantiates components just to call their `manifest` export. Chek's
/// own imports trap, which is safe because the contract calls `manifest`
/// before anything else and without network access.
struct ManifestReader {
    engine: Engine,
}

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl ManifestReader {
    fn new() -> Result<Self, String> {
        Ok(Self {
            engine: Engine::default(),
        })
    }

    fn read(&self, wasm: &[u8]) -> Result<Entry, String> {
        let component = Component::new(&self.engine, wasm)
            .map_err(|e| format!("isn't a WebAssembly component: {e}"))?;
        let mut linker = Linker::<State>::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|e| e.to_string())?;
        linker
            .define_unknown_imports_as_traps(&component)
            .map_err(|e| e.to_string())?;
        let mut store = Store::new(
            &self.engine,
            State {
                wasi: WasiCtx::builder().build(),
                table: ResourceTable::new(),
            },
        );
        let instance = linker
            .instantiate(&mut store, &component)
            .map_err(|e| format!("couldn't start it: {e}"))?;
        let manifest = instance
            .get_func(&mut store, "manifest")
            .ok_or("it doesn't export `manifest`; is it built with chek-extension?")?;
        let mut results = [Val::Bool(false)];
        manifest
            .call(&mut store, &[], &mut results)
            .map_err(|e| format!("its manifest failed: {e}"))?;
        let [Val::Record(fields)] = results else {
            return Err("its manifest isn't a record".to_owned());
        };
        Entry::from_manifest(fields)
    }
}

impl Entry {
    fn from_manifest(fields: Vec<(String, Val)>) -> Result<Self, String> {
        let mut entry = Entry {
            id: String::new(),
            name: String::new(),
            version: String::new(),
            file: String::new(),
            sha256: String::new(),
            sites: Vec::new(),
            languages: Vec::new(),
            capabilities: Vec::new(),
        };
        for (name, value) in fields {
            match (name.as_str(), value) {
                ("id", Val::String(s)) => entry.id = s,
                ("name", Val::String(s)) => entry.name = s,
                ("version", Val::String(s)) => entry.version = s,
                ("sites", Val::List(items)) => entry.sites = strings(items)?,
                ("languages", Val::List(items)) => entry.languages = strings(items)?,
                // Enum cases carry their WIT names, which are what indexes
                // list: `browser`, `browser-session`, `image-transform`.
                ("capabilities", Val::List(items)) => entry.capabilities = strings(items)?,
                _ => {}
            }
        }
        if entry.id.is_empty() || entry.name.is_empty() || entry.version.is_empty() {
            return Err("its manifest needs an id, name and version".to_owned());
        }
        Ok(entry)
    }
}

fn strings(items: Vec<Val>) -> Result<Vec<String>, String> {
    items
        .into_iter()
        .map(|item| match item {
            Val::String(s) | Val::Enum(s) => Ok(s),
            other => Err(format!("unexpected {other:?} in its manifest")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What this writes is what the server reads.
    #[test]
    fn indexes_read_as_the_server_reads_them() {
        let index = Index {
            spec: 1,
            name: Some("Mine".to_owned()),
            extensions: vec![Entry {
                id: "com.example.mine".to_owned(),
                name: "Mine".to_owned(),
                version: "0.1.0".to_owned(),
                file: "mine-0.1.0.wasm".to_owned(),
                sha256: "a".repeat(64),
                sites: vec!["example.com".to_owned()],
                languages: vec!["en".to_owned()],
                capabilities: vec!["browser".to_owned()],
            }],
        };
        let json = serde_json::to_vec(&index).unwrap();
        let read = chek_sources::repository::parse_index(&json).unwrap();
        assert_eq!(read.spec, Some(chek_sources::repository::SPEC_VERSION));
        assert_eq!(read.extensions[0].capabilities, ["browser"]);
    }

    #[test]
    fn folders_without_components_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(build(dir.path(), None).unwrap_err().contains("no .wasm"));
    }
}
