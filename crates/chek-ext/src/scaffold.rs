//! `chek-ext new`: a source crate that builds as it stands.

use std::path::Path;

const CARGO_TOML: &str = include_str!("templates/Cargo.toml.txt");
const LIB_RS: &str = include_str!("templates/lib.rs.txt");
const GITIGNORE: &str = "target/\ndist/\n.chek/\n";

pub fn new(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Err(format!(
            "{name:?} can't be a crate name; use lowercase letters, digits and dashes, such as my-source"
        ));
    }
    let dir = Path::new(name);
    if dir.exists() {
        return Err(format!("{name} already exists"));
    }
    let title = name
        .split('-')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let fill = |template: &str| {
        template
            .replace("{name}", name)
            .replace("{id}", &name.replace('-', "_"))
            .replace("{title}", &title)
    };
    let write = |path: &str, contents: &str| {
        let path = dir.join(path);
        std::fs::write(&path, contents)
            .map_err(|e| format!("couldn't write {}: {e}", path.display()))
    };
    std::fs::create_dir_all(dir.join("src")).map_err(|e| format!("couldn't create {name}: {e}"))?;
    write("Cargo.toml", &fill(CARGO_TOML))?;
    write("src/lib.rs", &fill(LIB_RS))?;
    write(".gitignore", GITIGNORE)?;
    println!("Created {name}. Next:\n  cd {name}\n  chek-ext build\n  chek-ext deploy cloudflare");
    Ok(())
}
