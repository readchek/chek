//! Writes a repository's `index.json` from the components in its folder,
//! reading each one's own manifest so the index can't disagree with what
//! it ships.

use std::path::Path;

use std::cmp::Ordering;

use serde::Serialize;
use sha2::{Digest, Sha256};
use wasmtime::component::{Component, Linker, ResourceTable, Val};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

/// A copy of `chek_sources::CONTRACTS`, which a test keeps the same: each
/// contract version with the first Chek release that runs it. Copied so
/// installing the CLI doesn't build the server's sources crate and libvips.
const CONTRACTS: &[(&str, &str)] = &[("0.6.0", "0.5.3")];

/// Orders release versions by number, as servers compare `minServer`.
fn compare_versions(a: &str, b: &str) -> Ordering {
    let numbers = |v: &str| -> Option<Vec<u64>> {
        v.split(['-', '+'])
            .next()?
            .split('.')
            .map(|n| n.parse().ok())
            .collect()
    };
    numbers(a).cmp(&numbers(b))
}

/// Version 1 of `docs/extension-repositories.md`.
#[derive(Debug, Serialize)]
pub struct Index {
    pub spec: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub extensions: Vec<Entry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub file: String,
    pub sha256: String,
    pub sites: Vec<String>,
    pub languages: Vec<String>,
    pub capabilities: Vec<String>,
    /// Left out for sources that read only images, as indexes written
    /// before formats existed are.
    #[serde(skip_serializing_if = "is_images")]
    pub formats: Vec<String>,
    /// The orders readers' comments come in, left out for sources that
    /// give none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<Vec<String>>,
    /// The first Chek release whose contract the component was built
    /// against.
    pub min_server: String,
    /// Older builds in the folder that need an older server than this
    /// one, newest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub previous: Vec<Entry>,
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
        println!(
            "{} {} ({}, needs Chek {})",
            entry.id, entry.version, entry.file, entry.min_server
        );
        extensions.push(entry);
    }
    Ok(Index {
        spec: 1,
        name: name.map(str::to_owned),
        extensions: newest_first(extensions),
    })
}

/// Lists each extension once, as its newest build, with the newest build
/// for each older server release it can still serve as `previous`. Other
/// builds are left out, since a server always takes the newest it runs.
fn newest_first(entries: Vec<Entry>) -> Vec<Entry> {
    let mut groups: Vec<Vec<Entry>> = Vec::new();
    for entry in entries {
        match groups.iter_mut().find(|g| g[0].id == entry.id) {
            Some(group) => group.push(entry),
            None => groups.push(vec![entry]),
        }
    }
    groups
        .into_iter()
        .map(|mut builds| {
            builds.sort_by(|a, b| compare_versions(&b.version, &a.version));
            let mut builds = builds.into_iter();
            let mut newest = builds.next().expect("groups aren't empty");
            for build in builds {
                let oldest_kept = newest.previous.last().unwrap_or(&newest);
                if compare_versions(&build.min_server, &oldest_kept.min_server).is_lt() {
                    newest.previous.push(build);
                } else {
                    println!(
                        "Leaving out {} {}: {} runs on the same servers",
                        build.id, build.version, oldest_kept.version
                    );
                }
            }
            newest
        })
        .collect()
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
        let mut entry = Entry::from_manifest(fields)?;
        let contract = contract(&self.engine, &component)?;
        entry.min_server = CONTRACTS
            .iter()
            .find(|(version, _)| *version == contract)
            .map(|(_, release)| *release)
            .ok_or_else(|| {
                format!(
                    "it's built against version {contract} of Chek's source contract, which this chek doesn't know; update chek-cli"
                )
            })?
            .to_owned();
        // Formats come from the optional chapter content export; a source
        // without it reads images.
        let formats = instance
            .get_export_index(&mut store, None, "chek:source/chapter-content@0.6.0")
            .and_then(|interface| {
                instance.get_export_index(&mut store, Some(&interface), "formats")
            })
            .and_then(|index| instance.get_func(&mut store, index));
        if let Some(formats) = formats {
            let mut results = [Val::Bool(false)];
            formats
                .call(&mut store, &[], &mut results)
                .map_err(|e| format!("its formats failed: {e}"))?;
            let [Val::List(items)] = results else {
                return Err("its formats aren't a list".to_owned());
            };
            entry.formats = strings(items)?;
            entry.formats.sort();
            entry.formats.dedup();
            if entry.formats.is_empty() {
                return Err("it lists no formats".to_owned());
            }
        }
        // Comment orders come from the optional chapter comments export,
        // in the extension's order, since the first is the site's default.
        let orders = instance
            .get_export_index(&mut store, None, "chek:source/chapter-comments@0.6.0")
            .and_then(|interface| instance.get_export_index(&mut store, Some(&interface), "orders"))
            .and_then(|index| instance.get_func(&mut store, index));
        if let Some(orders) = orders {
            let mut results = [Val::Bool(false)];
            orders
                .call(&mut store, &[], &mut results)
                .map_err(|e| format!("its comment orders failed: {e}"))?;
            let [Val::List(items)] = results else {
                return Err("its comment orders aren't a list".to_owned());
            };
            let mut kept: Vec<String> = Vec::new();
            for order in strings(items)? {
                if !kept.contains(&order) {
                    kept.push(order);
                }
            }
            entry.comments = Some(kept);
        }
        Ok(entry)
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
            formats: vec!["images".to_owned()],
            comments: None,
            min_server: String::new(),
            previous: Vec::new(),
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

/// The version of Chek's source contract a component was built against,
/// read from the `chek:source/…@version` interfaces it imports and exports.
fn contract(engine: &Engine, component: &Component) -> Result<String, String> {
    let ty = component.component_type();
    let mut versions: Vec<String> = ty
        .imports(engine)
        .map(|(name, _)| name)
        .chain(ty.exports(engine).map(|(name, _)| name))
        .filter_map(|name| name.strip_prefix("chek:source/"))
        .filter_map(|name| name.split_once('@').map(|(_, version)| version.to_owned()))
        .collect();
    versions.sort();
    versions.dedup();
    match &versions[..] {
        [version] => Ok(version.clone()),
        [] => Err("it uses no chek:source interface; is it built with chek-extension?".to_owned()),
        _ => Err(format!(
            "it mixes versions {versions:?} of Chek's source contract"
        )),
    }
}

fn is_images(formats: &[String]) -> bool {
    formats == ["images"]
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
                formats: vec!["epub".to_owned()],
                comments: Some(vec!["top".to_owned(), "newest".to_owned()]),
                min_server: "0.5.3".to_owned(),
                previous: Vec::new(),
            }],
        };
        let json = serde_json::to_vec(&index).unwrap();
        let read = chek_sources::repository::parse_index(&json).unwrap();
        assert_eq!(read.spec, Some(chek_sources::repository::SPEC_VERSION));
        assert_eq!(read.extensions[0].capabilities, ["browser"]);
        assert_eq!(read.extensions[0].formats, ["epub"]);
        assert_eq!(
            read.extensions[0].comments.as_deref(),
            Some(&["top".to_owned(), "newest".to_owned()][..])
        );
    }

    #[test]
    fn contracts_match_the_server() {
        assert_eq!(CONTRACTS, chek_sources::CONTRACTS);
    }

    /// Servers take the newest build they run, so a build is kept only for
    /// servers too old for every newer one.
    #[test]
    fn older_builds_are_kept_for_older_servers() {
        let build = |version: &str, min_server: &str| Entry {
            id: "com.example.mine".to_owned(),
            name: "Mine".to_owned(),
            version: version.to_owned(),
            file: format!("mine-{version}.wasm"),
            sha256: "a".repeat(64),
            sites: Vec::new(),
            languages: Vec::new(),
            capabilities: Vec::new(),
            formats: vec!["images".to_owned()],
            comments: None,
            min_server: min_server.to_owned(),
            previous: Vec::new(),
        };
        let listed = newest_first(vec![
            build("1.0.0", "0.5.3"),
            build("1.1.0", "0.5.3"),
            build("2.0.0", "0.7.0"),
            build("1.10.0", "0.6.0"),
        ]);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].version, "2.0.0");
        let previous: Vec<&str> = listed[0]
            .previous
            .iter()
            .map(|b| b.version.as_str())
            .collect();
        assert_eq!(previous, ["1.10.0", "1.1.0"]);

        let json = serde_json::to_vec(&Index {
            spec: 1,
            name: None,
            extensions: listed,
        })
        .unwrap();
        let read = chek_sources::repository::parse_index(&json).unwrap();
        assert_eq!(read.extensions[0].min_server.as_deref(), Some("0.7.0"));
        assert_eq!(
            read.extensions[0].previous[1].min_server.as_deref(),
            Some("0.5.3")
        );
    }

    #[test]
    fn folders_without_components_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(build(dir.path(), None).unwrap_err().contains("no .wasm"));
    }
}
