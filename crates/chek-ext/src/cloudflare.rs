//! `chek-ext deploy cloudflare`: the repository folder as a Cloudflare
//! Worker's static assets, behind a bearer token unless it's public.
//!
//! The Worker's files go in `.chek/cloudflare`, and Cloudflare's own
//! `wrangler` CLI does the deploying, so signing in and choosing an account
//! work the way Cloudflare documents them.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const WORKER: &str = include_str!("templates/worker.js.txt");
const SECRET: &str = "CHEK_REPOSITORY_TOKEN";

pub struct Deploy {
    pub dir: PathBuf,
    pub worker: Option<String>,
    pub public: bool,
    pub rotate_token: bool,
}

pub fn deploy(options: &Deploy) -> Result<(), String> {
    if !options.dir.join("index.json").is_file() {
        return Err(format!(
            "{} has no index.json; run `chek-ext build` first",
            options.dir.display()
        ));
    }
    let assets = options
        .dir
        .canonicalize()
        .map_err(|e| format!("couldn't find {}: {e}", options.dir.display()))?;
    let worker = match &options.worker {
        Some(name) => name.clone(),
        None => default_worker_name()?,
    };
    let config_dir = Path::new(".chek/cloudflare");
    std::fs::create_dir_all(config_dir).map_err(|e| format!("couldn't create .chek: {e}"))?;
    let config = config_dir.join("wrangler.json");
    let mut settings = serde_json::json!({
        "name": worker,
        "compatibility_date": "2026-10-01",
        "assets": { "directory": assets },
    });
    if !options.public {
        std::fs::write(config_dir.join("worker.js"), WORKER)
            .map_err(|e| format!("couldn't write the Worker: {e}"))?;
        settings["main"] = "worker.js".into();
        settings["assets"]["binding"] = "ASSETS".into();
        settings["assets"]["run_worker_first"] = true.into();
    }
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())? + "\n";
    std::fs::write(&config, json)
        .map_err(|e| format!("couldn't write {}: {e}", config.display()))?;

    let output = wrangler(&["deploy", "--config", path_str(&config)?], None)?;
    let url = output
        .split_whitespace()
        .find(|word| word.starts_with("https://") && word.contains(".workers.dev"))
        .map(str::to_owned);

    // The token is set once and kept by Cloudflare. A marker per Worker
    // remembers that, so deploying an update doesn't change it.
    let marker = config_dir.join(format!("{worker}.token-set"));
    let token = if options.public || (marker.exists() && !options.rotate_token) {
        None
    } else {
        let token = new_token();
        wrangler(
            &["secret", "put", SECRET, "--config", path_str(&config)?],
            Some(&token),
        )?;
        std::fs::write(&marker, "")
            .map_err(|e| format!("couldn't write {}: {e}", marker.display()))?;
        Some(token)
    };

    let address = url.unwrap_or_else(|| format!("https://{worker}.<your subdomain>.workers.dev"));
    println!("\nThe repository is at {address}");
    println!("Add it in Chek: Sources, Add Repository, then that address.");
    if options.public {
        println!("It's public: anyone with the address can read it.");
    } else if let Some(token) = token {
        println!("Sign in with a token: {token}");
        println!("This is the only time it's shown. Run with --rotate-token to replace it.");
    } else {
        println!("Its token is unchanged. Run with --rotate-token to replace it.");
    }
    Ok(())
}

/// Runs Cloudflare's CLI, from the PATH or through npx, and returns what it
/// printed after showing it.
fn wrangler(args: &[&str], stdin: Option<&str>) -> Result<String, String> {
    let installed = Command::new("wrangler")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let mut command = if installed {
        Command::new("wrangler")
    } else {
        let mut npx = Command::new("npx");
        npx.args(["--yes", "wrangler@4"]);
        npx
    };
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    command.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::inherit()
    });
    let mut child = command.spawn().map_err(|e| {
        format!("couldn't run wrangler ({e}); install Node.js, or wrangler itself with `npm install -g wrangler`")
    })?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes())
            .map_err(|e| format!("couldn't pass the token to wrangler: {e}"))?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let printed = String::from_utf8_lossy(&output.stdout).into_owned();
    // Never echo what `secret put` printed alongside a token being set.
    if stdin.is_none() {
        print!("{printed}");
    }
    if !output.status.success() {
        return Err(format!(
            "wrangler {} failed",
            args.first().copied().unwrap_or_default()
        ));
    }
    Ok(printed)
}

/// 32 random bytes as hex, which Chek sends as `Authorization: Bearer`.
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system provides randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The current folder's name, as Workers accept it: lowercase letters,
/// digits and dashes.
fn default_worker_name() -> Result<String, String> {
    let dir = std::env::current_dir().map_err(|e| e.to_string())?;
    let name: String = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let name = name.trim_matches('-').to_owned();
    if name.is_empty() {
        return Err("name the Worker with --worker".to_owned());
    }
    Ok(format!("{name}-chek"))
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("{} isn't UTF-8", path.display()))
}
