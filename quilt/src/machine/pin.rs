//! Pinned machines: named by content, and run with nothing else.
//!
//! `py⟨M⟩` means "whatever `python3` this shell would run", which is the right
//! default and the wrong thing to *record*: a cached answer, a logged run or a
//! reproduced result has to say which Python answered, built how, with which
//! libraries. A [`Pin`] is that answer. It names a Nix store path by the
//! **content address** of its whole runtime closure, and [`qspawn_pinned`]
//! starts the language's registered machine from that path with an empty
//! environment, so the pin alone decides what runs. The machine then reports
//! the address as its [`identity`](super::Machine::identity).
//!
//! # The address
//!
//! `nix store make-content-addressed` rewrites a store path, and everything it
//! references, to paths whose names are functions of their contents. The
//! rewritten path's name — `7r699cyp…-python3-3.14.10` — is the address. Nix
//! computes it and can verify it, it covers the closure rather than one file,
//! and it needs no experimental features.
//!
//! It is one address per platform: the same recipe built for another system is
//! other bytes and another address. Relating the two is a claim about a build,
//! which is somebody else's job.
//!
//! # What runs
//!
//! On Linux the rewritten path runs, so what runs is what was named. On macOS
//! rewriting a store path inside a signed binary invalidates its signature and
//! the kernel kills the process, and re-signing would change the content. So
//! there the *original* path runs, after `make-content-addressed` has been seen
//! to map it to the address. The gap is exactly the rewritten store paths
//! inside the files, and nothing else.
//!
//! Computing an address takes tens of seconds for a large closure, so the
//! mapping from original path to address is cached (`$QUILT_PIN_CACHE`, else
//! `quilt/pins` under the user's cache directory).
//!
//! Pinning says *which* machine ran, not what it was allowed to touch: a
//! pinned interpreter still runs with the caller's privileges.

use super::{repl_spec, script_spec, Machine, ReplMachine, ScriptMachine};
use crate::prelude::*;
use miette::{bail, miette, IntoDiagnostic, WrapErr};
use std::path::{Path, PathBuf};
use std::process::Command;

const STORE: &str = "/nix/store";

/// A machine's interpreter, pinned: its content address and the store path
/// that runs for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    /// The content address: the name of the content-addressed store path,
    /// such as `7r699cypjyibyji6c8l9yada8wbpv20a-python3-3.14.10`.
    pub address: Box<str>,
    /// The store path that runs: the content-addressed one, or on macOS the
    /// original it was computed from (see the [module docs](self)).
    pub path: PathBuf,
}

/// Whether `s` has the shape of a store path's name: a 32-character base-32
/// hash, a dash, a name.
#[must_use]
pub fn is_address(s: &str) -> bool {
    let Some((hash, name)) = s.split_once('-') else {
        return false;
    };
    // Nix's base-32 alphabet omits `e`, `o`, `t` and `u`.
    hash.len() == 32
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b.is_ascii_lowercase() && !b"eotu".contains(&b)))
        && !name.is_empty()
        && !name.contains('/')
}

fn nix(args: &[&str]) -> Result<String> {
    let out = Command::new("nix")
        .args(["--extra-experimental-features", "nix-command flakes"])
        .args(args)
        .output()
        .into_diagnostic()
        .wrap_err("failed to run `nix` (pinning a machine needs Nix)")?;
    if !out.status.success() {
        bail!(
            "nix {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn name_of(path: &Path) -> Result<&str> {
    path.file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| miette!("{} is not a store path", path.display()))
}

fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("QUILT_PIN_CACHE") {
        return dir.into();
    }
    let base = std::env::var_os("XDG_CACHE_HOME").map_or_else(
        || PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"),
        PathBuf::from,
    );
    base.join("quilt").join("pins")
}

/// The path `original` was rewritten to, read out of
/// `make-content-addressed --json`'s `{"rewrites":{"<original>":"<rewritten>"}}`.
/// Store paths hold no quotes or escapes, so this needs no JSON parser — which
/// keeps one off the runtime-only build.
fn rewritten<'j>(json: &'j str, original: &str) -> Option<&'j str> {
    let key = format!("\"{original}\"");
    let rest = json[json.find(&key)? + key.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start().strip_prefix('"')?;
    rest.split('"').next()
}

/// The content address of an original store path, from the cache or from Nix.
fn content_address(original: &Path) -> Result<Box<str>> {
    let entry = cache_dir().join(name_of(original)?);
    if let Ok(cached) = std::fs::read_to_string(&entry) {
        if is_address(cached.trim()) {
            return Ok(cached.trim().into());
        }
    }
    let path = original.to_string_lossy();
    let json = nix(&["store", "make-content-addressed", "--json", &path])?;
    let address = rewritten(&json, &path)
        .and_then(|p| Path::new(p).file_name()?.to_str())
        .filter(|a| is_address(a))
        .ok_or_else(|| miette!("nix rewrote nothing for {path}: {json}"))?;
    // The cache is an optimization: a read-only home must not fail a pin.
    if std::fs::create_dir_all(cache_dir()).is_ok() {
        let _ = std::fs::write(&entry, format!("{address}\n"));
    }
    Ok(address.into())
}

impl Pin {
    /// The path that runs for `address`, given the original it came from.
    fn runnable(address: Box<str>, original: PathBuf) -> Pin {
        let path = if cfg!(target_os = "macos") {
            original
        } else {
            Path::new(STORE).join(&*address)
        };
        Pin { address, path }
    }

    /// Pin a path already in the Nix store: compute its content address.
    ///
    /// # Errors
    /// If Nix is missing, or cannot rewrite the path.
    pub fn of_store_path(original: &Path) -> Result<Pin> {
        let address = content_address(original)?;
        Ok(Pin::runnable(address, original.to_path_buf()))
    }

    /// Build `recipe` — an installable such as `nixpkgs#python3`, or better a
    /// locked `github:NixOS/nixpkgs/<rev>#python3` — and pin what it builds.
    ///
    /// The recipe is only a way to *get* the machine. Two recipes that build
    /// the same bytes give the same pin, and the address is what to record.
    ///
    /// # Errors
    /// If Nix is missing, the recipe doesn't build, or it builds to more than
    /// one path.
    pub fn build(recipe: &str) -> Result<Pin> {
        let out = nix(&["build", recipe, "--no-link", "--print-out-paths"])?;
        let mut paths = out.lines();
        let (Some(original), None) = (paths.next(), paths.next()) else {
            bail!("{recipe} builds to {} paths, not one", out.lines().count());
        };
        Pin::of_store_path(Path::new(original))
    }

    /// Find what runs for a recorded `address`, and confirm it.
    ///
    /// With a recipe, build it and refuse it if it gives another address.
    /// Without one, use what an earlier pin cached, or on Linux the
    /// content-addressed path itself if the store has it.
    ///
    /// # Errors
    /// If `address` is malformed, the recipe builds to another address, or
    /// nothing in the local store realizes it.
    pub fn check(address: &str, recipe: Option<&str>) -> Result<Pin> {
        if !is_address(address) {
            bail!("{address:?} is not a Nix content address");
        }
        if let Some(recipe) = recipe {
            let pin = Pin::build(recipe)?;
            if &*pin.address != address {
                bail!("{recipe} builds to {}, not {address}", pin.address);
            }
            return Ok(pin);
        }
        // An original whose cached address is this one, if it is still there.
        for entry in std::fs::read_dir(cache_dir())
            .into_iter()
            .flatten()
            .flatten()
        {
            let original = Path::new(STORE).join(entry.file_name());
            let cached = std::fs::read_to_string(entry.path()).unwrap_or_default();
            if cached.trim() == address && original.exists() {
                return Ok(Pin::runnable(address.into(), original));
            }
        }
        let rewritten = Path::new(STORE).join(address);
        if !cfg!(target_os = "macos") && rewritten.exists() {
            // The store checked the name against the contents when it took
            // the path, so its presence is the confirmation.
            return Ok(Pin {
                address: address.into(),
                path: rewritten,
            });
        }
        bail!("no machine with address {address} is realized here; pass the recipe that builds it")
    }

    /// The pinned spelling of an interpreter a spec names by bare name:
    /// `<path>/bin/<program>`.
    fn program(&self, program: &str) -> Result<Box<str>> {
        let pinned = self.path.join("bin").join(program);
        if !pinned.exists() {
            bail!(
                "the machine pinned at {} has no bin/{program}",
                self.address
            );
        }
        Ok(pinned.to_string_lossy().into())
    }
}

/// Spawn `lang`'s registered machine from a [`Pin`]: the same provider
/// [`qspawn`](super::qspawn) would choose, with its interpreter taken from the
/// pinned path instead of `PATH`, an environment holding only what the spec
/// sets, and the pin's address as its [`identity`](Machine::identity).
///
/// # Errors
/// If `lang` has no subprocess machine (a native machine has no interpreter
/// to pin), the pin holds no such interpreter, or it fails to start.
pub fn qspawn_pinned(lang: &str, pin: &Pin) -> Result<Box<dyn Machine>> {
    if let Some(mut spec) = repl_spec(lang) {
        spec.program = pin.program(&spec.program)?;
        spec.clear_env = true;
        spec.identity = Some(pin.address.clone());
        return Ok(Box::new(ReplMachine::spawn(lang, spec)?));
    }
    if let Some(mut spec) = script_spec(lang) {
        spec.program = pin.program(&spec.program)?;
        spec.clear_env = true;
        spec.identity = Some(pin.address.clone());
        return Ok(Box::new(ScriptMachine::new(lang, spec)));
    }
    bail!("no subprocess machine is registered for {lang:?}, so there is nothing to pin")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::InnerKind;
    use crate::qterm::qleaf;

    #[test]
    fn an_address_is_a_store_paths_name() {
        assert!(is_address(
            "7r699cypjyibyji6c8l9yada8wbpv20a-python3-3.14.10"
        ));
        assert!(!is_address("Python 3.14.10"));
        assert!(!is_address("7r699cypjyibyji6c8l9yada8wbpv20a"));
        // `e` is not in Nix's alphabet.
        assert!(!is_address("7r699cypjyibyji6c8l9yada8wbpv20e-python3"));
        assert!(!is_address(
            "/nix/store/7r699cypjyibyji6c8l9yada8wbpv20a-python3"
        ));
    }

    #[test]
    fn the_rewrite_is_read_from_nixs_json() {
        let json = r#"{"rewrites":{"/nix/store/aaa-x":"/nix/store/bbb-x"}}"#;
        assert_eq!(
            rewritten(json, "/nix/store/aaa-x"),
            Some("/nix/store/bbb-x")
        );
        assert_eq!(rewritten(json, "/nix/store/ccc-x"), None);
    }

    #[test]
    fn an_unknown_address_is_refused_before_anything_runs() {
        let e = Pin::check("00000000000000000000000000000000-nothing", None).unwrap_err();
        assert!(e.to_string().contains("recipe"), "{e}");
        assert!(Pin::check("python3", None).is_err());
    }

    /// The whole path, against a real store: build, address, spawn, answer.
    /// Run with `cargo test -p quiltlang pin -- --ignored`.
    #[test]
    #[ignore = "builds nixpkgs#python3 with Nix, which needs the network"]
    fn a_pinned_python_answers_and_says_which_it_is() {
        let pin = Pin::build("nixpkgs#python3").unwrap();
        assert!(is_address(&pin.address));
        // The address is a function of the contents, so it is stable, and a
        // recorded address finds its machine again without the recipe.
        assert_eq!(Pin::check(&pin.address, None).unwrap(), pin);
        assert!(Pin::check(&pin.address, Some("nixpkgs#bash")).is_err());

        let mut m = qspawn_pinned("py", &pin).unwrap();
        assert_eq!(m.identity(), Some(&*pin.address));
        m.feed(InnerKind::Item, &qleaf("code", "x = 20")).unwrap();
        let answer = m.eval(&qleaf("code", "x + 22")).unwrap();
        assert_eq!(answer.value.as_deref(), Some("42"));
        // Nothing of the caller's environment reached it.
        let path = m
            .eval(&qleaf("code", "__import__('os').environ.get('PATH')"))
            .unwrap();
        assert_eq!(path.value.as_deref(), Some("None"));
        // And it is the pinned interpreter, not the one on `PATH`.
        let exe = m
            .eval(&qleaf("code", "__import__('sys').executable"))
            .unwrap();
        assert!(exe.value.unwrap().contains(STORE));
    }
}
