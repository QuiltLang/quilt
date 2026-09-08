//! Machines: stateful evaluators of a language.
//!
//! A [`Machine`] is the semantic twin of [`Language`](crate::lang::Language):
//! where a `Language` says what a fragment *is* (text → term), a `Machine`
//! says what it *does*, and remembers — fed programs over time, it
//! accumulates definitions and effects, and answers queries with the literal
//! spelling of the value. See `docs/design/machines.md` for the full design
//! (meta-machines, `↓` on machines, REPL/Jupyter/DB providers); this module
//! is phase 1: the trait, the [`Answer`], the replay-based [`ScriptMachine`],
//! and the [`Park`] of live per-language defaults that
//! [`Multi`](crate::multi::Multi) carries.
//!
//! The wire is text, not trees (tenet 2): [`Machine::feed_str`] is the
//! primitive, and the term-level conveniences coparse first, so a machine
//! implementation never touches `QTerm` internals. Parsing an answer back
//! into a term is the `Multi` rim's job
//! ([`eval_on`](crate::multi::Multi::eval_on)), because that is where the
//! `Language` lives. Like `lift`, this module has no tree-sitter dependency
//! and is part of the runtime-only build.

use crate::lang::InnerKind;
use crate::prelude::*;
use crate::qterm::QTerm;
use miette::{bail, IntoDiagnostic, WrapErr};
use std::collections::BTreeMap;
use std::process::Command;

/**************************************************************/

/// What a [`ScriptMachine`] needs to know about a language: how to run a
/// script of it, and how to spell "print this value as a literal".
///
/// Declared per language via
/// [`Language::machine_spec`](crate::lang::Language::machine_spec), the same
/// pattern as [`hashbang`](crate::lang::Language::hashbang) — and usually
/// derived from it, via [`MachineSpec::from_hashbang`].
#[derive(Debug, Clone)]
pub struct MachineSpec {
    /// The interpreter to execute, e.g. `python3`.
    pub program: Box<str>,
    /// Arguments that precede the script path, e.g.
    /// `--experimental-strip-types` for node.
    pub args: Box<[Box<str>]>,
    /// The spelling of "print the value of this expression as a literal of
    /// this language", with `{}` standing for the expression — e.g. Python's
    /// `print(repr({}))`. This is what makes an [`Answer::value`] the inverse
    /// of lift: the machine answers with source that re-denotes the value.
    pub print_wrap: Box<str>,
    /// Suffix for the temp script file, e.g. `.py` (some runners sniff it).
    pub suffix: Box<str>,
    /// Environment variables set for the interpreter (overriding inherited
    /// values) — e.g. the `PYTHONPATH` that makes `from quilt import *`
    /// resolve, the same path `reduce_py` teaches its one-shot script.
    pub env: Box<[(Box<str>, Box<str>)]>,
}

impl MachineSpec {
    /// Derive the runner from a shebang line (unwrapping `env` the way
    /// [`parse_hashbang`](crate::lang::parse_hashbang) does), supplying the
    /// two spellings the shebang cannot: the print wrapper and the suffix.
    ///
    /// `None` when the shebang names no interpreter.
    #[must_use]
    pub fn from_hashbang(hashbang: &str, print_wrap: &str, suffix: &str) -> Option<MachineSpec> {
        let (program, args) = crate::lang::parse_hashbang(hashbang)?;
        Some(MachineSpec {
            program: program.into(),
            args: args.iter().map(|a| Box::from(*a)).collect(),
            print_wrap: print_wrap.into(),
            suffix: suffix.into(),
            env: Box::default(),
        })
    }
}

/**************************************************************/

/// A machine's reply to one fed fragment.
#[derive(Debug, Clone, Default)]
pub struct Answer {
    /// The value as a literal of the machine's own language — the inverse of
    /// lift — when the fragment was a query ([`InnerKind::Expr`]) and the
    /// value has a literal spelling. `None` for definitions and effects.
    pub value: Option<Box<str>>,
    /// A residue: when a value has no literal spelling (an open file, a
    /// closure), the machine binds it to a fresh name and answers with the
    /// reference instead. The value stays in the machine. (No phase-1
    /// provider produces residues yet; the field is the protocol's.)
    pub name: Option<Box<str>>,
    /// Everything the feed wrote to stdout, for display and diagnostics.
    /// For a replay-based machine this includes the replayed history's
    /// output, not only the new fragment's.
    pub stdout: Box<str>,
    /// Everything the feed wrote to stderr.
    pub stderr: Box<str>,
}

/**************************************************************/

/// A stateful evaluator of one language.
///
/// The message sorts are [`InnerKind`] — already the vocabulary "for
/// communicating between parsers" (`lang.rs`), and equally the vocabulary
/// for communicating with machines:
///
/// * [`Item`](InnerKind::Item) — a definition: extends the environment.
/// * [`Stmt`](InnerKind::Stmt) — an effect: runs against the environment.
/// * [`Expr`](InnerKind::Expr) — a query: answers the value's denotation.
/// * [`File`](InnerKind::File) / [`Block`](InnerKind::Block) — a batch.
///
/// The laws a well-behaved machine satisfies (asserted by the conformance
/// battery as providers grow): *denotation* — evaluating a literal answers
/// that literal; *sequencing* — `feed(p1); feed(p2)` behaves as feeding
/// `p1; p2` to a fresh machine; *isolation* — two machines share nothing.
///
/// `Send` is a supertrait because parked machines travel with their `Multi`
/// (test harnesses hold one behind a `Mutex`); a machine is a handle to a
/// process or connection, which sends fine.
pub trait Machine: Send {
    /// The language this machine speaks (a registry key, e.g. `"py"`).
    fn lang(&self) -> &str;

    /// Feed one fragment of source text. Definitions and effects accumulate;
    /// queries answer. The textual primitive every other method wraps.
    fn feed_str(&mut self, kind: InnerKind, src: &str) -> Result<Answer>;

    /// Feed a term: coparse, then [`feed_str`](Machine::feed_str).
    fn feed(&mut self, kind: InnerKind, term: &QTerm) -> Result<Answer> {
        self.feed_str(kind, &term.coparse())
    }

    /// Query a term's value: [`feed`](Machine::feed) as [`InnerKind::Expr`].
    fn eval(&mut self, term: &QTerm) -> Result<Answer> {
        self.feed(InnerKind::Expr, term)
    }
}

/**************************************************************/

/// The degenerate machine: no live process, state as replayed history.
///
/// Definitions and effects are buffered; every feed writes
/// `history + fragment` to a temp script and runs it through a fresh
/// interpreter process ([`MachineSpec`]), so definitions accumulate across
/// feeds even though nothing persists between processes. This recovers
/// today's one-shot `↓` as the zero-infrastructure default — with the honest
/// cost that *effects in the history re-run on every feed*, so it is only
/// faithful for effect-free preludes. A persistent provider (REPL, Jupyter,
/// DB — see the design doc) lifts that restriction behind the same trait.
///
/// A query's answer is the last non-empty stdout line of the run, produced by
/// the spec's `print_wrap` spelling. A fragment whose run fails does not
/// enter the history.
pub struct ScriptMachine {
    lang: Box<str>,
    spec: MachineSpec,
    /// Every successfully fed non-query fragment, in feed order.
    history: Vec<Box<str>>,
}

impl ScriptMachine {
    #[must_use]
    pub fn new(lang: &str, spec: MachineSpec) -> ScriptMachine {
        ScriptMachine {
            lang: lang.into(),
            spec,
            history: Vec::new(),
        }
    }

    /// The replayed prelude plus `frag`, as one script.
    fn script_with(&self, frag: &str) -> String {
        let mut script = String::new();
        for fed in &self.history {
            script.push_str(fed);
            script.push('\n');
        }
        script.push_str(frag);
        script.push('\n');
        script
    }

    /// Write `script` to a temp file and run it through the spec's
    /// interpreter, capturing both streams. A non-zero exit is an error
    /// carrying the captured stderr.
    fn run(&self, script: &str) -> Result<(Box<str>, Box<str>)> {
        let file = tempfile::Builder::new()
            .suffix(&*self.spec.suffix)
            .tempfile()
            .into_diagnostic()?;
        std::fs::write(file.path(), script).into_diagnostic()?;
        let output = Command::new(&*self.spec.program)
            .args(self.spec.args.iter().map(AsRef::<str>::as_ref))
            .envs(self.spec.env.iter().map(|(k, v)| (&**k, &**v)))
            .arg(file.path())
            .output()
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to run {:?}", self.spec.program))?;
        let stdout: Box<str> = String::from_utf8_lossy(&output.stdout).into();
        let stderr: Box<str> = String::from_utf8_lossy(&output.stderr).into();
        if !output.status.success() {
            bail!(
                "{} machine: {:?} failed with {}:\n{stderr}",
                self.lang,
                self.spec.program,
                output.status
            );
        }
        Ok((stdout, stderr))
    }
}

impl Machine for ScriptMachine {
    fn lang(&self) -> &str {
        &self.lang
    }

    fn feed_str(&mut self, kind: InnerKind, src: &str) -> Result<Answer> {
        if kind == InnerKind::Expr {
            let printed = self.spec.print_wrap.replace("{}", src);
            let (stdout, stderr) = self.run(&self.script_with(&printed))?;
            let value = stdout
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .map(|line| Box::from(line.trim()));
            Ok(Answer {
                value,
                name: None,
                stdout,
                stderr,
            })
        } else {
            let (stdout, stderr) = self.run(&self.script_with(src))?;
            self.history.push(src.into());
            Ok(Answer {
                value: None,
                name: None,
                stdout,
                stderr,
            })
        }
    }
}

/**************************************************************/

/// What a [`ReplMachine`] needs to know about a language: how to start its
/// interactive interpreter, how to spell a value query, and how to spell
/// "print exactly this text" — the sentinel that marks where one feed's
/// output ends.
///
/// Declared per language via
/// [`Language::repl_spec`](crate::lang::Language::repl_spec), preferred over
/// [`machine_spec`](crate::lang::Language::machine_spec) when both exist.
#[derive(Debug, Clone)]
pub struct ReplSpec {
    /// The interpreter to run and keep alive, e.g. `bash`.
    pub program: Box<str>,
    pub args: Box<[Box<str>]>,
    /// The spelling of "print the value of this expression as a literal",
    /// with `{}` standing for the expression — for the shells,
    /// `echo $(( {} ))` (a shell query is an arithmetic expression).
    pub print_wrap: Box<str>,
    /// The spelling of "print exactly this text on its own line", with `{}`
    /// standing for the text. Fed after every fragment with a fresh sentinel;
    /// the machine reads output until the sentinel comes back, which is how
    /// one feed's output is separated from the next without any framing
    /// support from the interpreter.
    pub echo_wrap: Box<str>,
    /// Environment variables set for the interpreter (overriding inherited
    /// values); see [`MachineSpec::env`].
    pub env: Box<[(Box<str>, Box<str>)]>,
}

/// How long a [`ReplMachine`] waits for a feed's sentinel before declaring
/// the interpreter hung.
const REPL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A persistent machine: one long-lived interpreter process, fed fragments
/// on stdin. State is real process state — definitions persist because the
/// process does — so, unlike [`ScriptMachine`], effects run exactly once and
/// nothing is replayed.
///
/// The wire protocol is the least an interactive interpreter can offer:
/// write the fragment, write an `echo` of a fresh sentinel, read stdout
/// until the sentinel comes back. An interpreter that dies (or never echoes
/// within [`REPL_TIMEOUT`]) turns the feed into an error carrying whatever
/// stderr said. This is the phase-3 provider from `docs/design/machines.md`;
/// it fits any interpreter whose input is a statement stream (the shells,
/// `sqlite3`), while block-structured REPLs (python's `...` continuations)
/// need the richer per-language protocols that come later.
pub struct ReplMachine {
    lang: Box<str>,
    spec: ReplSpec,
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    /// Lines the reader thread has pulled off the interpreter's stdout.
    stdout: std::sync::mpsc::Receiver<String>,
    /// Everything stderr has said since the last feed (drained per feed).
    stderr: std::sync::Arc<std::sync::Mutex<String>>,
    /// Feed counter, salting the sentinel.
    feeds: usize,
}

impl ReplMachine {
    /// Start the interpreter and attach the stream readers.
    pub fn spawn(lang: &str, spec: ReplSpec) -> Result<ReplMachine> {
        use std::io::{BufRead as _, BufReader};

        let mut child = Command::new(&*spec.program)
            .args(spec.args.iter().map(AsRef::<str>::as_ref))
            .envs(spec.env.iter().map(|(k, v)| (&**k, &**v)))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to start {:?}", spec.program))?;
        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout_pipe = child.stdout.take().expect("stdout was piped");
        let stderr_pipe = child.stderr.take().expect("stderr was piped");

        let (tx, stdout) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout_pipe).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });

        let stderr = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let sink = std::sync::Arc::clone(&stderr);
        std::thread::spawn(move || {
            for line in BufReader::new(stderr_pipe).lines() {
                let Ok(line) = line else { break };
                if let Ok(mut buf) = sink.lock() {
                    buf.push_str(&line);
                    buf.push('\n');
                }
            }
        });

        Ok(ReplMachine {
            lang: lang.into(),
            spec,
            child,
            stdin,
            stdout,
            stderr,
            feeds: 0,
        })
    }

    /// Take whatever stderr has said since the last drain.
    fn drain_stderr(&self) -> Box<str> {
        self.stderr
            .lock()
            .map(|mut buf| std::mem::take(&mut *buf))
            .unwrap_or_default()
            .into()
    }
}

impl Machine for ReplMachine {
    fn lang(&self) -> &str {
        &self.lang
    }

    fn feed_str(&mut self, kind: InnerKind, src: &str) -> Result<Answer> {
        use std::io::Write as _;
        use std::sync::mpsc::RecvTimeoutError;

        self.feeds += 1;
        let sentinel = format!("__QUILT_REPL_DONE_{}__", self.feeds);
        let payload = if kind == InnerKind::Expr {
            self.spec.print_wrap.replace("{}", src)
        } else {
            src.to_string()
        };
        let input = format!(
            "{payload}\n{}\n",
            self.spec.echo_wrap.replace("{}", &sentinel)
        );
        if let Err(e) = self
            .stdin
            .write_all(input.as_bytes())
            .and_then(|()| self.stdin.flush())
        {
            bail!(
                "{} machine: {:?} is gone ({e}):\n{}",
                self.lang,
                self.spec.program,
                self.drain_stderr()
            );
        }

        let mut lines = Vec::new();
        loop {
            match self.stdout.recv_timeout(REPL_TIMEOUT) {
                Ok(line) if line.trim() == sentinel => break,
                Ok(line) => lines.push(line),
                Err(RecvTimeoutError::Timeout) => bail!(
                    "{} machine: {:?} did not answer within {REPL_TIMEOUT:?}",
                    self.lang,
                    self.spec.program
                ),
                Err(RecvTimeoutError::Disconnected) => bail!(
                    "{} machine: {:?} exited mid-feed:\n{}",
                    self.lang,
                    self.spec.program,
                    self.drain_stderr()
                ),
            }
        }

        let value = (kind == InnerKind::Expr)
            .then(|| {
                lines
                    .iter()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .map(|line| Box::from(line.trim()))
            })
            .flatten();
        Ok(Answer {
            value,
            name: None,
            stdout: lines.join("\n").into(),
            stderr: self.drain_stderr(),
        })
    }
}

impl Drop for ReplMachine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/**************************************************************/

/// Spawn the best machine a [`Language`](crate::lang::Language) declares: the
/// persistent [`ReplMachine`] when the language has a
/// [`repl_spec`](crate::lang::Language::repl_spec), else the replay-based
/// [`ScriptMachine`] from its
/// [`machine_spec`](crate::lang::Language::machine_spec), else an error.
/// Shared by [`Multi`](crate::multi::Multi) and the conformance battery, so
/// the machine a claim is verified against is the machine users get.
pub fn spawn_machine<L: crate::lang::Language + ?Sized>(
    lang_name: &str,
    lang: &L,
) -> Result<Box<dyn Machine>> {
    if let Some(spec) = lang.repl_spec() {
        return Ok(Box::new(ReplMachine::spawn(lang_name, spec)?));
    }
    if let Some(spec) = lang.machine_spec() {
        return Ok(Box::new(ScriptMachine::new(lang_name, spec)));
    }
    bail!("language {lang_name:?} has no machine: no repl or script spec registered")
}

/**************************************************************/

/// The pool of live default machines, one per language, spawned on first
/// use. [`Multi`](crate::multi::Multi) carries one so that successive
/// reduces in a run share definitions; `quilt run` will keep it warm for the
/// run's duration (phase 2).
#[derive(Default)]
pub struct Park {
    machines: BTreeMap<Box<str>, Box<dyn Machine>>,
}

impl Park {
    #[must_use]
    pub fn contains(&self, lang: &str) -> bool {
        self.machines.contains_key(lang)
    }

    /// Adopt `machine` as the default for `lang`, replacing any previous one.
    pub fn insert(&mut self, lang: &str, machine: Box<dyn Machine>) {
        self.machines.insert(lang.into(), machine);
    }

    /// The default machine for `lang`, if one has been parked.
    pub fn get_mut(&mut self, lang: &str) -> Option<&mut (dyn Machine + 'static)> {
        self.machines.get_mut(lang).map(AsMut::as_mut)
    }
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    /// POSIX sh as a machine: definitions are assignments, queries are
    /// arithmetic. Hermetic on every dev platform, no language feature
    /// needed.
    fn sh_machine() -> ScriptMachine {
        ScriptMachine::new(
            "sh",
            MachineSpec {
                program: "sh".into(),
                args: Box::default(),
                print_wrap: "echo $(( {} ))".into(),
                suffix: ".sh".into(),
                env: Box::default(),
            },
        )
    }

    #[test]
    fn definitions_accumulate_and_queries_answer() {
        let mut m = sh_machine();
        let fed = m.feed_str(InnerKind::Item, "x=5").unwrap();
        assert_eq!(fed.value, None);
        let ask = m.feed_str(InnerKind::Expr, "x + 2").unwrap();
        assert_eq!(ask.value.as_deref(), Some("7"));
        // The environment survives the query: ask again.
        let ask = m.feed_str(InnerKind::Expr, "x * x").unwrap();
        assert_eq!(ask.value.as_deref(), Some("25"));
    }

    #[test]
    fn machines_are_isolated() {
        let mut a = sh_machine();
        let mut b = sh_machine();
        a.feed_str(InnerKind::Item, "x=1").unwrap();
        b.feed_str(InnerKind::Item, "x=100").unwrap();
        assert_eq!(
            a.feed_str(InnerKind::Expr, "x + 1")
                .unwrap()
                .value
                .as_deref(),
            Some("2")
        );
        assert_eq!(
            b.feed_str(InnerKind::Expr, "x + 1")
                .unwrap()
                .value
                .as_deref(),
            Some("101")
        );
    }

    #[test]
    fn failed_feeds_stay_out_of_history() {
        let mut m = sh_machine();
        m.feed_str(InnerKind::Item, "x=5").unwrap();
        assert!(m.feed_str(InnerKind::Stmt, "exit 3").is_err());
        // The failed fragment was not recorded, so the machine still answers.
        let ask = m.feed_str(InnerKind::Expr, "x + 2").unwrap();
        assert_eq!(ask.value.as_deref(), Some("7"));
    }

    /// POSIX sh as a *persistent* machine, for the [`ReplMachine`] tests.
    fn sh_repl() -> ReplMachine {
        ReplMachine::spawn(
            "sh",
            ReplSpec {
                program: "sh".into(),
                args: Box::default(),
                print_wrap: "echo $(( {} ))".into(),
                echo_wrap: "echo {}".into(),
                env: Box::default(),
            },
        )
        .unwrap()
    }

    #[test]
    fn repl_state_persists_without_replay() {
        let mut m = sh_repl();
        let fed = m.feed_str(InnerKind::Stmt, "echo hi").unwrap();
        assert!(fed.stdout.contains("hi"));
        m.feed_str(InnerKind::Item, "x=5").unwrap();
        let ask = m.feed_str(InnerKind::Expr, "x + 2").unwrap();
        assert_eq!(ask.value.as_deref(), Some("7"));
        // The effect ran exactly once: the query's own output does not
        // re-print "hi" — a live process, not replayed history.
        assert!(!ask.stdout.contains("hi"), "got: {:?}", ask.stdout);
    }

    #[test]
    fn a_dead_repl_is_an_error() {
        let mut m = sh_repl();
        assert!(m.feed_str(InnerKind::Stmt, "exit 0").is_err());
    }

    /// The spec `PythonProvider` registers, exercised without the `parse`
    /// feature (the tuple is kept in sync by the `Multi`-level test in
    /// `multi.rs`, which goes through the provider).
    #[test]
    fn python_answers_its_own_literals() {
        let spec =
            MachineSpec::from_hashbang("#!/usr/bin/env python3", "print(repr({}))", ".py").unwrap();
        let mut m = ScriptMachine::new("py", spec);
        m.feed_str(InnerKind::Item, "def f(x):\n    return x * 2")
            .unwrap();
        assert_eq!(
            m.feed_str(InnerKind::Expr, "f(21)")
                .unwrap()
                .value
                .as_deref(),
            Some("42")
        );
        // repr() answers a *literal*: a string comes back quoted.
        assert_eq!(
            m.feed_str(InnerKind::Expr, "'ab' * 2")
                .unwrap()
                .value
                .as_deref(),
            Some("'abab'")
        );
    }
}
