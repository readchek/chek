//! `chek ext deploy cloudflare`: the repository folder as a Cloudflare
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
            "{} has no index.json; run `chek ext build` first",
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

    let config_arg = path_str(&config)?;
    let output = wrangler(&["deploy", "--config", config_arg], Input::None, Echo::Yes)?;
    let url = output
        .split_whitespace()
        .find(|word| word.starts_with("https://") && word.contains(".workers.dev"))
        .map(str::to_owned);

    // The token is set once and kept by Cloudflare, so deploying an update,
    // from this machine or another, leaves it alone. Cloudflare is asked
    // whether it has one rather than anything kept here.
    let has_token = !options.public && has_secret(config_arg);
    let token = if options.public || (has_token && !options.rotate_token) {
        None
    } else {
        let token = new_token();
        wrangler(
            &["secret", "put", SECRET, "--config", config_arg],
            Input::Secret(&token),
            Echo::No,
        )?;
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

enum Input<'a> {
    None,
    /// Passed on stdin, never as an argument, so it stays out of process
    /// lists and shell history.
    Secret(&'a str),
}

#[derive(PartialEq)]
enum Echo {
    Yes,
    No,
}

/// Whether the Worker already has its token. A Worker that doesn't exist
/// yet, or a listing that fails, counts as no.
fn has_secret(config: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Secret {
        name: String,
    }
    wrangler(
        &["secret", "list", "--format", "json", "--config", config],
        Input::None,
        Echo::No,
    )
    .ok()
    .and_then(|output| serde_json::from_str::<Vec<Secret>>(output.trim()).ok())
    .is_some_and(|secrets| secrets.iter().any(|s| s.name == SECRET))
}

/// Runs Cloudflare's CLI, from the PATH or through npx, and returns what it
/// printed. With `Echo::Yes` its output is shown line by line as it runs,
/// so a sign-in link appears while wrangler waits on it.
fn wrangler(args: &[&str], input: Input<'_>, echo: Echo) -> Result<String, String> {
    use std::io::BufRead;

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
        .stderr(if echo == Echo::Yes {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .stdin(match input {
            Input::None => Stdio::inherit(),
            Input::Secret(_) => Stdio::piped(),
        });
    let mut child = command.spawn().map_err(|e| {
        format!(
            "couldn't run wrangler ({e}); install Node.js, or wrangler itself with `npm install -g wrangler`"
        )
    })?;
    if let (Input::Secret(text), Some(mut pipe)) = (input, child.stdin.take()) {
        pipe.write_all(text.as_bytes())
            .map_err(|e| format!("couldn't pass the token to wrangler: {e}"))?;
    }
    let mut printed = String::new();
    if let Some(stdout) = child.stdout.take() {
        for line in std::io::BufReader::new(stdout).lines() {
            let line = line.map_err(|e| e.to_string())?;
            if echo == Echo::Yes {
                println!("{line}");
            }
            printed.push_str(&line);
            printed.push('\n');
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
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
