//! `chek-ext`: makes Chek extension repositories and puts them online.
//!
//! The repository format is in `docs/extension-repositories.md`. This tool
//! only writes that format and drives the host's own CLI; nothing it makes
//! needs it to keep running.

mod cloudflare;
mod compile;
mod index;
mod scaffold;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
chek-ext: build Chek extensions and publish them as a repository

Usage:
  chek-ext new <name>                     Start a source crate in ./<name>
  chek-ext build [--out dist] [--name N]  Build every source here into a repository folder
  chek-ext index <folder> [--name N]      Write index.json for a folder of components
  chek-ext deploy cloudflare [options]    Put the folder on Cloudflare Workers

Deploy options:
  --dir <folder>     The repository folder (default dist)
  --worker <name>    The Worker's name (default: the current folder's name)
  --public           No token; anyone with the address can read it
  --rotate-token     Replace the token, which signs out every server using the old one

The repository format: https://github.com/readchek/chek/blob/main/docs/extension-repositories.md
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let (command, rest) = args.split_first().ok_or(USAGE)?;
    let mut options = Options::parse(rest)?;
    // Every argument is read before anything runs, so a mistyped flag
    // stops the command instead of being ignored by it.
    let action: Box<dyn FnOnce() -> Result<(), String>> = match command.as_str() {
        "new" => {
            let name = options.positional(0, "a name for the source")?;
            Box::new(move || scaffold::new(&name))
        }
        "build" => {
            let out = options.path("--out", "dist");
            let name = options.value("--name");
            Box::new(move || {
                compile::build(&out)?;
                index::write(&out, name.as_deref())
            })
        }
        "index" => {
            let dir = PathBuf::from(options.positional(0, "the folder of components")?);
            let name = options.value("--name");
            Box::new(move || index::write(&dir, name.as_deref()))
        }
        "deploy" => match options
            .positional(0, "where to deploy, such as cloudflare")?
            .as_str()
        {
            "cloudflare" => {
                let deploy = cloudflare::Deploy {
                    dir: options.path("--dir", "dist"),
                    worker: options.value("--worker"),
                    public: options.flag("--public"),
                    rotate_token: options.flag("--rotate-token"),
                };
                Box::new(move || cloudflare::deploy(&deploy))
            }
            other => {
                return Err(format!(
                    "can't deploy to {other:?} yet; Cloudflare is supported"
                ));
            }
        },
        "help" | "--help" | "-h" => Box::new(|| {
            print!("{USAGE}");
            Ok(())
        }),
        other => return Err(format!("unknown command {other:?}\n\n{USAGE}")),
    };
    options.finish()?;
    action()
}

/// Arguments after the command: `--flag`, `--key value` and positionals.
/// Each is taken once; any left over is an error, so typos don't pass
/// silently.
struct Options {
    positional: Vec<Option<String>>,
    named: Vec<(String, Option<String>)>,
}

/// Options that take a value; anything else starting with `--` is a flag.
const VALUED: &[&str] = &["--out", "--name", "--dir", "--worker"];

impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut options = Options {
            positional: Vec::new(),
            named: Vec::new(),
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if VALUED.contains(&arg.as_str()) {
                let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
                options.named.push((arg.clone(), Some(value.clone())));
            } else if arg.starts_with("--") {
                options.named.push((arg.clone(), None));
            } else {
                options.positional.push(Some(arg.clone()));
            }
        }
        Ok(options)
    }

    fn positional(&mut self, at: usize, what: &str) -> Result<String, String> {
        self.positional
            .get_mut(at)
            .and_then(Option::take)
            .ok_or_else(|| format!("give {what}\n\n{USAGE}"))
    }

    fn value(&mut self, name: &str) -> Option<String> {
        let at = self
            .named
            .iter()
            .position(|(n, v)| n == name && v.is_some())?;
        self.named.remove(at).1
    }

    fn path(&mut self, name: &str, default: &str) -> PathBuf {
        PathBuf::from(self.value(name).unwrap_or_else(|| default.to_owned()))
    }

    fn flag(&mut self, name: &str) -> bool {
        match self
            .named
            .iter()
            .position(|(n, v)| n == name && v.is_none())
        {
            Some(at) => {
                self.named.remove(at);
                true
            }
            None => false,
        }
    }

    fn finish(self) -> Result<(), String> {
        let unused: Vec<String> = self
            .positional
            .into_iter()
            .flatten()
            .chain(self.named.into_iter().map(|(name, _)| name))
            .collect();
        if unused.is_empty() {
            Ok(())
        } else {
            Err(format!("didn't understand {}", unused.join(" ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(args: &[&str]) -> Options {
        Options::parse(&args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn options_are_taken_once() {
        let mut parsed = options(&["cloudflare", "--dir", "out", "--public"]);
        assert_eq!(parsed.positional(0, "where").unwrap(), "cloudflare");
        assert_eq!(parsed.path("--dir", "dist"), PathBuf::from("out"));
        assert!(parsed.flag("--public"));
        assert!(!parsed.flag("--rotate-token"));
        assert!(parsed.finish().is_ok());
    }

    #[test]
    fn leftovers_are_reported() {
        let mut parsed = options(&["cloudflare", "--pubic"]);
        parsed.positional(0, "where").unwrap();
        assert!(!parsed.flag("--public"));
        assert!(parsed.finish().unwrap_err().contains("--pubic"));
        assert!(Options::parse(&["--out".to_owned()]).is_err());
    }
}
