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
//! The trait traffics in terms ([`Machine::feed`]), like every other Quilt
//! interface — tenet 2 is about the *end user* writing plain source text,
//! not about internal APIs. The validated textual doors live where the
//! `Language` that can parse lives: [`Multi::feed_on`] takes user text in,
//! and [`Multi::eval_on`] parses the answered literal back out. A provider
//! whose *wire* is text (a subprocess's stdin) coparses internally — its
//! business, not the caller's — and exposes that raw door as inherent
//! `feed_str`/`type_of_str` methods on the concrete type. Like `lift`, this
//! module has no tree-sitter dependency and is part of the runtime-only
//! build.
//!
//! [`Multi::feed_on`]: crate::multi::Multi::feed_on
//! [`Multi::eval_on`]: crate::multi::Multi::eval_on

use crate::lang::InnerKind;
use crate::prelude::*;
use crate::qterm::QTerm;
use miette::{bail, IntoDiagnostic, WrapErr};
use std::collections::BTreeMap;
use std::process::Command;

// The one *native* provider: a document held in-process, whose definitions
// are elements with ids. Term-only, so it belongs on this runtime path like
// the rest of the module.
pub mod html;
pub use html::HtmlMachine;

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
    /// The spelling of the *typing* judgment, when the language has one:
    /// "print the type of this expression", `{}` standing for the
    /// expression — python's `print(type({}).__name__)`, Lean's `#check {}`.
    /// `None` gives [`Machine::type_of`]'s honest default error.
    pub type_wrap: Option<Box<str>>,
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
            type_wrap: None,
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
///
/// The trait traffics in **terms**, like every other Quilt interface. Tenet
/// 2 is about the *end user* writing plain source text rather than builder
/// calls — it says nothing about internal APIs, and the parser that can turn
/// user text into a term lives with the `Language`, not here. So the
/// validated textual door is [`Multi::feed_on`](crate::multi::Multi::feed_on)
/// at the rim; a provider whose *wire* is text (a subprocess's stdin)
/// coparses internally, which is its business and no caller's.
pub trait Machine: Send {
    /// The language this machine speaks (a registry key, e.g. `"py"`).
    fn lang(&self) -> &str;

    /// Feed one term. Definitions and effects accumulate; queries answer.
    fn feed(&mut self, kind: InnerKind, term: &QTerm) -> Result<Answer>;

    /// Query a term's value: [`feed`](Machine::feed) as [`InnerKind::Expr`].
    fn eval(&mut self, term: &QTerm) -> Result<Answer> {
        self.feed(InnerKind::Expr, term)
    }

    /// The *typing* judgment: answer the type of the term, as a literal of
    /// this language's type spellings (`int`, `42 : ℕ`, `unsat`). An
    /// evaluator asks "what value"; a checker — Lean, a solver, an LSP —
    /// asks this instead, and a proof language's machine may support *only*
    /// this and Item-checking. Defaults to an honest error, the same
    /// pattern as the operator spellings on `MetaLanguage`.
    fn type_of(&mut self, term: &QTerm) -> Result<Answer> {
        let _ = term;
        bail!(
            "the {} machine has no typing judgment registered (no type_wrap in its spec)",
            self.lang()
        )
    }
}

/**************************************************************/

/// An opaque snapshot of one machine's state, for fork and rollback. The
/// encoding is the providing machine's own; restoring into a different
/// provider fails at decode rather than corrupting anything.
pub struct Snapshot(Box<[u8]>);

/// Machines that can checkpoint and roll back their environment — Racket
/// namespaces, Smalltalk images, a solver's `push`/`pop`, and (trivially,
/// exactly) a [`ScriptMachine`]'s history.
pub trait SnapshotMachine: Machine {
    fn snapshot(&self) -> Result<Snapshot>;
    fn restore(&mut self, snapshot: &Snapshot) -> Result<()>;
}

/// What a bounded run of a [`MeteredMachine`] came back with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunResult {
    Finished,
    Suspended,
}

/// Machines whose execution is resource-bounded and resumable — the shape of
/// nanobots' `StateMachine` (gas checked per basic block, explicit yields)
/// and of wasmtime's fuel. No provider in this crate implements it yet; the
/// trait is here so the downstream shape and the machine family agree.
pub trait MeteredMachine: Machine {
    fn add_gas(&mut self, amount: u32);
    fn run(&mut self, max_gas: u32) -> Result<RunResult>;
}

/// One definition a machine's environment holds.
pub struct Def {
    pub name: Box<str>,
    pub kind: InnerKind,
}

/// Machines that can enumerate what they hold — the machine-side symbol
/// table `quilt-lsp` wants to inject into projections (issue #193).
pub trait IntrospectMachine: Machine {
    fn defs(&self) -> Result<Vec<Def>>;
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
///
/// An [`Answer::stdout`] is what *this* feed printed: the replayed history
/// prints again on every run, and that prefix is stripped — the output of
/// a feed should not depend on which provider the language happens to have.
/// (History whose output varies between runs defeats the stripping; that is
/// the replay model's honest cost, not a framing bug.)
pub struct ScriptMachine {
    lang: Box<str>,
    spec: MachineSpec,
    /// Every successfully fed non-query fragment, in feed order.
    history: Vec<Box<str>>,
    /// What running the history alone printed, last time it was run — the
    /// prefix every later run's stdout starts with.
    replayed: Box<str>,
}

impl ScriptMachine {
    #[must_use]
    pub fn new(lang: &str, spec: MachineSpec) -> ScriptMachine {
        ScriptMachine {
            lang: lang.into(),
            spec,
            history: Vec::new(),
            replayed: Box::default(),
        }
    }

    /// `stdout` with the history's own output removed from the front.
    fn fresh(&self, stdout: &str) -> Box<str> {
        stdout
            .strip_prefix(&*self.replayed)
            .unwrap_or(stdout)
            .into()
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

impl ScriptMachine {
    /// Run `history + wrapped` and read the last non-empty stdout line as
    /// the answered literal — the shared shape of the value and typing
    /// queries.
    fn query(&self, wrapped: &str) -> Result<Answer> {
        let (stdout, stderr) = self.run(&self.script_with(wrapped))?;
        let value = stdout
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(|line| Box::from(line.trim()));
        Ok(Answer {
            value,
            name: None,
            stdout: self.fresh(&stdout),
            stderr,
        })
    }
}

impl ScriptMachine {
    /// The raw-text door on the concrete provider (this machine's wire *is*
    /// text). Callers holding a term go through [`Machine::feed`]; callers
    /// holding user-typed source go through
    /// [`Multi::feed_on`](crate::multi::Multi::feed_on), which parses first.
    pub fn feed_str(&mut self, kind: InnerKind, src: &str) -> Result<Answer> {
        if kind == InnerKind::Expr {
            self.query(&self.spec.print_wrap.replace("{}", src))
        } else {
            let (stdout, stderr) = self.run(&self.script_with(src))?;
            let fresh = self.fresh(&stdout);
            self.history.push(src.into());
            self.replayed = stdout;
            Ok(Answer {
                value: None,
                name: None,
                stdout: fresh,
                stderr,
            })
        }
    }

    /// Raw-text twin of [`Machine::type_of`].
    pub fn type_of_str(&mut self, expr: &str) -> Result<Answer> {
        let Some(wrap) = &self.spec.type_wrap else {
            bail!(
                "the {} machine has no typing judgment registered (no type_wrap in its spec)",
                self.lang
            )
        };
        self.query(&wrap.replace("{}", expr))
    }
}

impl Machine for ScriptMachine {
    fn lang(&self) -> &str {
        &self.lang
    }

    fn feed(&mut self, kind: InnerKind, term: &QTerm) -> Result<Answer> {
        self.feed_str(kind, term.coparse().trim())
    }

    fn type_of(&mut self, term: &QTerm) -> Result<Answer> {
        self.type_of_str(term.coparse().trim())
    }
}

/// A [`ScriptMachine`]'s whole state is its history, so snapshots are exact
/// and free: the replay model's one honest advantage over a live process.
impl SnapshotMachine for ScriptMachine {
    fn snapshot(&self) -> Result<Snapshot> {
        Ok(Snapshot(
            postcard::to_stdvec(&self.history).into_diagnostic()?.into(),
        ))
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<()> {
        self.history = postcard::from_bytes(&snapshot.0).into_diagnostic()?;
        Ok(())
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
    ///
    /// The protocol has one more bit: an interpreter that can tell the
    /// fragment *failed* echoes the sentinel followed by ` !` instead, and
    /// the feed is an error carrying what stderr said — the same contract
    /// the replay machine gets from a non-zero exit. The shells spell this
    /// off `$?`; the python and node kernels off the exception they caught.
    pub echo_wrap: Box<str>,
    /// Environment variables set for the interpreter (overriding inherited
    /// values); see [`MachineSpec::env`].
    pub env: Box<[(Box<str>, Box<str>)]>,
    /// The typing-judgment spelling; see [`MachineSpec::type_wrap`].
    pub type_wrap: Option<Box<str>>,
    /// The spelling of "print exactly this text on its own line **on
    /// stderr**", with `{}` standing for the text — `echo {} >&2` for the
    /// shells. When present, every feed is framed on both streams: the
    /// machine reads stderr up to the sentinel too, so an [`Answer::stderr`]
    /// is exactly what *this* feed said rather than whatever had arrived by
    /// the time stdout's sentinel did (a traceback's last lines used to be
    /// a race). `None` keeps the best-effort drain, for interpreters with
    /// no way to echo to stderr (`sqlite3`).
    pub echo_err_wrap: Option<Box<str>>,
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
    /// Lines the other reader thread has pulled off its stderr.
    stderr: std::sync::mpsc::Receiver<String>,
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

        // One reader thread per stream, each feeding a channel of lines.
        // Lines are read as bytes and converted lossily: an interpreter that
        // writes a byte sequence that is not UTF-8 (macOS's bash 3.2 does,
        // for a multibyte character right after a `$var`) must not end the
        // stream — `lines()` would, and the machine would then look dead.
        let lines_of = |pipe: Box<dyn std::io::Read + Send>| {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(pipe);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match reader.read_until(b'\n', &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    if buf.last() == Some(&b'\n') {
                        buf.pop();
                        if buf.last() == Some(&b'\r') {
                            buf.pop();
                        }
                    }
                    if tx.send(String::from_utf8_lossy(&buf).into_owned()).is_err() {
                        break;
                    }
                }
            });
            rx
        };
        let stdout = lines_of(Box::new(stdout_pipe));
        let stderr = lines_of(Box::new(stderr_pipe));

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

    /// Take whatever stderr has said so far — the best-effort answer, for a
    /// spec with no stderr echo, and for reporting a dead interpreter.
    fn drain_stderr(&self) -> Box<str> {
        let mut out = String::new();
        while let Ok(line) = self.stderr.try_recv() {
            out.push_str(&line);
            out.push('\n');
        }
        out.into()
    }

    /// Read stderr up to this feed's sentinel — exact, when the spec can
    /// echo to stderr.
    fn stderr_until(&self, sentinel: &str) -> Result<Box<str>> {
        use std::sync::mpsc::RecvTimeoutError;
        let mut out = String::new();
        loop {
            match self.stderr.recv_timeout(REPL_TIMEOUT) {
                Ok(line) if line.trim() == sentinel => return Ok(out.into()),
                Ok(line) => {
                    out.push_str(&line);
                    out.push('\n');
                }
                Err(RecvTimeoutError::Timeout) => bail!(
                    "{} machine: {:?} did not echo on stderr within {REPL_TIMEOUT:?}",
                    self.lang,
                    self.spec.program
                ),
                Err(RecvTimeoutError::Disconnected) => bail!(
                    "{} machine: {:?} exited mid-feed:\n{out}",
                    self.lang,
                    self.spec.program
                ),
            }
        }
    }
}

impl ReplMachine {
    /// One sentinel-framed exchange: write `payload`, read until the
    /// sentinel, answer with the collected output (`value` from the last
    /// non-empty line when `collect_value`).
    fn exchange(&mut self, payload: &str, collect_value: bool) -> Result<Answer> {
        use std::io::Write as _;
        use std::sync::mpsc::RecvTimeoutError;

        self.feeds += 1;
        let sentinel = format!("__QUILT_REPL_DONE_{}__", self.feeds);
        let mut input = format!(
            "{payload}\n{}\n",
            self.spec.echo_wrap.replace("{}", &sentinel)
        );
        if let Some(err_wrap) = &self.spec.echo_err_wrap {
            input.push_str(&err_wrap.replace("{}", &sentinel));
            input.push('\n');
        }
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

        let rejected = format!("{sentinel} !");
        let mut failed = false;
        let mut lines = Vec::new();
        loop {
            match self.stdout.recv_timeout(REPL_TIMEOUT) {
                Ok(line) if line.trim() == sentinel => break,
                Ok(line) if line.trim() == rejected => {
                    failed = true;
                    break;
                }
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

        let value = collect_value
            .then(|| {
                lines
                    .iter()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .map(|line| Box::from(line.trim()))
            })
            .flatten();
        let stderr = if self.spec.echo_err_wrap.is_some() {
            self.stderr_until(&sentinel)?
        } else {
            self.drain_stderr()
        };
        if failed {
            bail!(
                "{} machine: {:?} rejected the fragment:\n{stderr}",
                self.lang,
                self.spec.program
            );
        }
        Ok(Answer {
            value,
            name: None,
            stdout: lines.join("\n").into(),
            stderr,
        })
    }
}

impl ReplMachine {
    /// The raw-text door on the concrete provider; see
    /// [`ScriptMachine::feed_str`] for who belongs at which door.
    pub fn feed_str(&mut self, kind: InnerKind, src: &str) -> Result<Answer> {
        if kind == InnerKind::Expr {
            let payload = self.spec.print_wrap.replace("{}", src);
            self.exchange(&payload, true)
        } else {
            self.exchange(src, false)
        }
    }

    /// Raw-text twin of [`Machine::type_of`].
    pub fn type_of_str(&mut self, expr: &str) -> Result<Answer> {
        let Some(wrap) = &self.spec.type_wrap else {
            bail!(
                "the {} machine has no typing judgment registered (no type_wrap in its spec)",
                self.lang
            )
        };
        let payload = wrap.replace("{}", expr);
        self.exchange(&payload, true)
    }
}

impl Machine for ReplMachine {
    fn lang(&self) -> &str {
        &self.lang
    }

    fn feed(&mut self, kind: InnerKind, term: &QTerm) -> Result<Answer> {
        self.feed_str(kind, term.coparse().trim())
    }

    fn type_of(&mut self, term: &QTerm) -> Result<Answer> {
        self.type_of_str(term.coparse().trim())
    }
}

impl Drop for ReplMachine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/**************************************************************/

/// Every language quilt knows how to run as a machine, in table order.
///
/// The table is *data* — program names, print/echo wrappers, a suffix — and
/// carries no parsing knowledge, which is why it lives here on the
/// runtime-only path rather than behind the `parse` feature with the
/// `Language` registry (issue #273). Three consumers read it and cannot
/// disagree: [`Language::machine_spec`](crate::lang::Language::machine_spec)
/// and [`repl_spec`](crate::lang::Language::repl_spec) delegate into it, the
/// conformance battery drives the machine those return, and [`qspawn`] — what
/// `lang⟨M⟩` expands to in a Rust ground program, which has no registry at
/// all — reads it directly.
pub const REGISTERED_SPECS: &[&str] = &[
    "python",
    "py",
    "typescript",
    "ts",
    "sql",
    "bash",
    "zsh",
    "html",
];

/// Whether `lang` has a registered machine — native, persistent or replay.
///
/// A host's `spawn_str` asks before it spells `lang⟨M⟩`, so a machine no
/// provider backs is a diagnostic at expansion time — pointing at the glyph in
/// the source — rather than a failure inside generated code.
#[must_use]
pub fn has_spec(lang: &str) -> bool {
    native_machine(lang).is_some() || repl_spec(lang).is_some() || script_spec(lang).is_some()
}

/// The in-process machine registered for `lang`, if any: no interpreter,
/// state held as a term. HTML's is the [`HtmlMachine`] — a document whose
/// definitions are its ids. Preferred over both subprocess providers by
/// [`qspawn`] and [`spawn_machine`], since it is the only one that can
/// answer with the same term it was fed.
#[must_use]
pub fn native_machine(lang: &str) -> Option<Box<dyn Machine>> {
    match lang {
        "html" => Some(Box::new(HtmlMachine::default())),
        _ => None,
    }
}

/// The Python kernel behind the persistent python machine: a stdin loop
/// exec-ing each sentinel-framed chunk in one namespace. Framing is the
/// [`ReplSpec`]'s: a chunk ends at the line `echo_wrap` spells, so the
/// kernel matches that spelling (and `echo_err_wrap`'s) rather than parsing
/// Python incrementally — which is what lets `def`/`if`/`else` blocks cross
/// lines without the `...` continuation protocol a real REPL needs. The
/// quilt runtime is pre-imported when it is built, the way quilt-python's
/// `run()` pre-imports it into the namespace it returns.
const PYTHON_KERNEL: &str = r#"
import re, sys, traceback
ns = {"__name__": "__quilt__"}
try:
    exec("from quilt import *", ns)
except ImportError:
    pass
mark = re.compile(r'^print\("(__QUILT_REPL_DONE_\d+__)"(, file=sys\.stderr)?\)$')
buf = []
for line in sys.stdin:
    line = line.rstrip("\n")
    m = mark.match(line)
    if m is None:
        buf.append(line)
        continue
    if m.group(2):
        sys.stderr.write(m.group(1) + "\n")
        sys.stderr.flush()
        continue
    src = "\n".join(buf) + "\n"
    buf = []
    failed = ""
    try:
        exec(compile(src, "<quilt>", "exec"), ns)
    except Exception:
        traceback.print_exc()
        failed = " !"
    sys.stdout.flush()
    sys.stderr.flush()
    print(m.group(1) + failed)
    sys.stdout.flush()
"#;

/// The Node kernel behind the persistent typescript machine, the twin of
/// [`PYTHON_KERNEL`]: one `vm` context kept for the life of the process, so
/// a top-level `const` in one chunk is visible to the next (the REPL's
/// semantics). Type annotations are stripped on a syntax error, as the Node
/// runtime's `↓` does. The quilt-wasm runtime's builders are made globals of
/// the context when the package is built (`bin/build-ts`), so an expanded
/// `html↖…↗` in a chunk resolves without an `import` — which a `vm` script
/// could not have.
const TYPESCRIPT_KERNEL: &str = r#"
const vm = require("node:vm"), rl = require("node:readline"), mod = require("node:module");
const ctx = vm.createContext(Object.assign(Object.create(globalThis), { require, console, process }));
try { Object.assign(ctx, require(process.env.QUILT_WASM_PKG)); } catch {}
const mark = /^console\.(log|error)\("(__QUILT_REPL_DONE_\d+__)"\)$/;
let buf = [];
function run(src) {
  try {
    return vm.runInContext(src, ctx, { filename: "<quilt>" });
  } catch (e) {
    if (e?.name !== "SyntaxError" || typeof mod.stripTypeScriptTypes !== "function") throw e;
    let stripped;
    try { stripped = mod.stripTypeScriptTypes(src, { mode: "strip" }); } catch { throw e; }
    return vm.runInContext(stripped, ctx, { filename: "<quilt>" });
  }
}
rl.createInterface({ input: process.stdin, terminal: false }).on("line", (line) => {
  const m = mark.exec(line);
  if (!m) { buf.push(line); return; }
  if (m[1] === "error") { console.error(m[2]); return; }
  const src = buf.join("\n");
  buf = [];
  let failed = "";
  try { run(src); } catch (e) { console.error(e && e.stack ? e.stack : String(e)); failed = " !"; }
  console.log(m[2] + failed);
});
"#;

/// The persistent-REPL spec registered for `lang`, if any. See
/// [`REGISTERED_SPECS`]; the per-provider reasoning lives on the
/// [`Language::repl_spec`](crate::lang::Language::repl_spec) impl that
/// delegates here.
#[must_use]
pub fn repl_spec(lang: &str) -> Option<ReplSpec> {
    let (program, args, print_wrap, echo_wrap, echo_err_wrap, type_wrap): (
        _,
        &[&str],
        _,
        _,
        Option<&str>,
        _,
    ) = match lang {
        // `-batch` so sqlite3 reads stdin without its interactive banner, and
        // `-bail` so a failed statement is an error rather than silence.
        "sql" => (
            "sqlite3",
            &["-batch", "-bail"],
            "SELECT {};",
            "SELECT '{}';",
            None,
            Some("SELECT typeof({});"),
        ),
        // A shell query is an arithmetic expression, so that is what "print
        // the value of this expression" means for both shells. The echo
        // reads `$?` first: a fragment whose last command failed is
        // rejected, which is what a non-zero status means to a shell.
        "bash" => (
            "bash",
            &[],
            "echo $(( {} ))",
            "[ $? -eq 0 ] && echo {} || echo '{} !'",
            Some("echo {} >&2"),
            None,
        ),
        "zsh" => (
            "zsh",
            &[],
            "echo $(( {} ))",
            "[ $? -eq 0 ] && echo {} || echo '{} !'",
            Some("echo {} >&2"),
            None,
        ),
        // `-u`: unbuffered, so a chunk's output precedes its sentinel. The
        // echo spellings are the ones the kernel's regex matches.
        "python" | "py" => (
            "python3",
            &["-u", "-c", PYTHON_KERNEL],
            "print(repr({}))",
            "print(\"{}\")",
            Some("print(\"{}\", file=sys.stderr)"),
            Some("print(type({}).__name__)"),
        ),
        // `--no-warnings`: stripping types is experimental in some Node
        // versions, and the warning would land in the first feed's stderr.
        "typescript" | "ts" => (
            "node",
            &["--no-warnings", "-e", TYPESCRIPT_KERNEL],
            "console.log(JSON.stringify({}))",
            "console.log(\"{}\")",
            Some("console.error(\"{}\")"),
            Some("console.log(typeof ({}))"),
        ),
        _ => return None,
    };
    let env: Box<[(Box<str>, Box<str>)]> = match lang {
        // The `PYTHONPATH` that makes `from quilt import *` resolve — the
        // same path the script spec and `reduce_py` teach.
        "python" | "py" => Box::new([(
            "PYTHONPATH".into(),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../quilt-python").into(),
        )]),
        // Where the kernel finds the quilt-wasm runtime, when built; and no
        // ANSI colour in `console.log` output, whatever the caller's
        // terminal says (`FORCE_COLOR` outranks `NO_COLOR` in Node).
        "typescript" | "ts" => Box::new([
            (
                "QUILT_WASM_PKG".into(),
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../quilt-wasm/pkg/quilt_wasm.js"
                )
                .into(),
            ),
            ("FORCE_COLOR".into(), "0".into()),
        ]),
        _ => Box::default(),
    };
    Some(ReplSpec {
        program: program.into(),
        args: args.iter().map(|a| Box::from(*a)).collect(),
        print_wrap: print_wrap.into(),
        echo_wrap: echo_wrap.into(),
        env,
        type_wrap: type_wrap.map(Box::from),
        echo_err_wrap: echo_err_wrap.map(Box::from),
    })
}

/// The replay-script spec registered for `lang`, if any. See
/// [`REGISTERED_SPECS`].
#[must_use]
pub fn script_spec(lang: &str) -> Option<MachineSpec> {
    match lang {
        "python" | "py" => {
            let mut spec =
                MachineSpec::from_hashbang("#!/usr/bin/env python3", "print(repr({}))", ".py")?;
            // The `PYTHONPATH` that makes `from quilt import *` resolve — the
            // same path `reduce_py` teaches its one-shot script.
            spec.env = Box::new([(
                "PYTHONPATH".into(),
                concat!(env!("CARGO_MANIFEST_DIR"), "/../quilt-python").into(),
            )]);
            // The typing judgment: `type_of("21 + 21")` answers `int`.
            spec.type_wrap = Some("print(type({}).__name__)".into());
            Some(spec)
        }
        "typescript" | "ts" => MachineSpec::from_hashbang(
            "#!/usr/bin/env -S node --experimental-strip-types",
            "console.log(JSON.stringify({}))",
            ".ts",
        ),
        _ => None,
    }
}

/// Spawn a machine for `lang` from the registered table, preferring native,
/// then persistent, then replay — what `lang⟨M⟩` expands to in Rust
/// (issue #273).
///
/// The registry-driven [`spawn_machine`] is the same choice made through a
/// [`Language`](crate::lang::Language); this one needs no registry, so it is
/// available to a Rust ground program built runtime-only
/// (`default-features = false`), which is how expanded `.rs.quilt` files are
/// built.
pub fn qspawn(lang: &str) -> Result<Box<dyn Machine>> {
    if let Some(machine) = native_machine(lang) {
        return Ok(machine);
    }
    if let Some(spec) = repl_spec(lang) {
        return Ok(Box::new(ReplMachine::spawn(lang, spec)?));
    }
    if let Some(spec) = script_spec(lang) {
        return Ok(Box::new(ScriptMachine::new(lang, spec)));
    }
    bail!(
        "no machine is registered for {lang:?}; quilt knows: {}",
        REGISTERED_SPECS.join(", ")
    )
}

/**************************************************************/

/// Spawn the best machine a [`Language`](crate::lang::Language) declares: its
/// [`native_machine`](crate::lang::Language::native_machine) when it has
/// one, else the persistent [`ReplMachine`] when the language has a
/// [`repl_spec`](crate::lang::Language::repl_spec), else the replay-based
/// [`ScriptMachine`] from its
/// [`machine_spec`](crate::lang::Language::machine_spec), else an error.
/// Shared by [`Multi`](crate::multi::Multi) and the conformance battery, so
/// the machine a claim is verified against is the machine users get.
pub fn spawn_machine<L: crate::lang::Language + ?Sized>(
    lang_name: &str,
    lang: &L,
) -> Result<Box<dyn Machine>> {
    if let Some(machine) = lang.native_machine() {
        return Ok(machine);
    }
    if let Some(spec) = lang.repl_spec() {
        return Ok(Box::new(ReplMachine::spawn(lang_name, spec)?));
    }
    if let Some(spec) = lang.machine_spec() {
        return Ok(Box::new(ScriptMachine::new(lang_name, spec)));
    }
    bail!("language {lang_name:?} has no machine: no native, repl or script provider registered")
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
                type_wrap: None,
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

    /// A query does not re-print what the history printed: the replayed
    /// prefix is stripped, so stdout is this feed's own.
    #[test]
    fn replayed_output_is_not_reported_again() {
        let mut m = sh_machine();
        let fed = m.feed_str(InnerKind::Stmt, "echo hi").unwrap();
        assert_eq!(&*fed.stdout, "hi\n");
        let ask = m.feed_str(InnerKind::Expr, "1 + 1").unwrap();
        assert_eq!(ask.value.as_deref(), Some("2"));
        assert_eq!(&*ask.stdout, "2\n", "got: {:?}", ask.stdout);
        let fed = m.feed_str(InnerKind::Stmt, "echo again").unwrap();
        assert_eq!(&*fed.stdout, "again\n");
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
                type_wrap: None,
                echo_err_wrap: Some("echo {} >&2".into()),
            },
        )
        .unwrap()
    }

    /// With a stderr echo, what a feed said on stderr is exactly this feed's
    /// — read up to the sentinel, not whatever had arrived.
    #[test]
    fn stderr_is_framed_per_feed() {
        let mut m = sh_repl();
        let a = m.feed_str(InnerKind::Stmt, "echo oops >&2").unwrap();
        assert_eq!(&*a.stderr, "oops\n");
        let b = m.feed_str(InnerKind::Expr, "1 + 1").unwrap();
        assert_eq!(&*b.stderr, "", "the next feed starts clean");
    }

    /// The registered shells report a failed fragment as an error — the
    /// sentinel's one extra bit — and stay usable afterwards.
    #[test]
    fn bash_rejects_a_failing_fragment() {
        let mut m = ReplMachine::spawn("bash", repl_spec("bash").unwrap()).unwrap();
        m.feed_str(InnerKind::Item, "x=5").unwrap();
        let err = m.feed_str(InnerKind::Stmt, "false").unwrap_err();
        assert!(err.to_string().contains("rejected"), "{err}");
        let err = m
            .feed_str(InnerKind::Stmt, "echo boom >&2; exit_code_of_nothing")
            .unwrap_err();
        assert!(err.to_string().contains("boom"), "carries stderr: {err}");
        assert_eq!(
            m.feed_str(InnerKind::Expr, "x + 1")
                .unwrap()
                .value
                .as_deref(),
            Some("6")
        );
    }

    /// The python kernel: one process, definitions persist, blocks cross
    /// lines, an exception is reported on stderr and leaves the kernel
    /// usable, and effects run exactly once.
    #[test]
    fn python_kernel_is_a_persistent_machine() {
        let mut m = ReplMachine::spawn("py", repl_spec("py").unwrap()).unwrap();
        let fed = m.feed_str(InnerKind::Stmt, "print('once')").unwrap();
        assert_eq!(&*fed.stdout, "once");
        m.feed_str(
            InnerKind::Item,
            "def f(x):\n    if x > 1:\n        return x * 2\n    else:\n        return x",
        )
        .unwrap();
        let ask = m.feed_str(InnerKind::Expr, "f(21)").unwrap();
        assert_eq!(ask.value.as_deref(), Some("42"));
        assert!(!ask.stdout.contains("once"), "no replay: {:?}", ask.stdout);
        let bad = m.feed_str(InnerKind::Expr, "undefined_name").unwrap_err();
        assert!(bad.to_string().contains("NameError"), "{bad}");
        let ask = m.feed_str(InnerKind::Expr, "f(1)").unwrap();
        assert_eq!(ask.value.as_deref(), Some("1"), "still alive");
        assert_eq!(
            m.type_of_str("f(21)").unwrap().value.as_deref(),
            Some("int")
        );
    }

    /// The node kernel, held to the same shape.
    #[test]
    fn typescript_kernel_is_a_persistent_machine() {
        let mut m = ReplMachine::spawn("ts", repl_spec("ts").unwrap()).unwrap();
        m.feed_str(InnerKind::Item, "const y: number = 40").unwrap();
        let ask = m.feed_str(InnerKind::Expr, "y + 2").unwrap();
        assert_eq!(ask.value.as_deref(), Some("42"));
        let bad = m.feed_str(InnerKind::Expr, "nope").unwrap_err();
        assert!(bad.to_string().contains("ReferenceError"), "{bad}");
        let ask = m.feed_str(InnerKind::Expr, "[y, 'a']").unwrap();
        assert_eq!(ask.value.as_deref(), Some("[40,\"a\"]"));
        assert_eq!(m.type_of_str("y").unwrap().value.as_deref(), Some("number"));
    }

    /// The native provider wins the spawn order, and answers with the term
    /// it was fed.
    #[test]
    fn html_is_native() {
        let mut m = qspawn("html").unwrap();
        assert_eq!(m.lang(), "html");
        let ask = m.feed(InnerKind::Expr, &leaf("text", "hi")).unwrap();
        assert_eq!(ask.value.as_deref(), Some("hi"));
        assert!(has_spec("html"));
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

    /// A snapshot is the whole state of a script machine, so restore rolls
    /// definitions back exactly.
    #[test]
    fn snapshots_roll_definitions_back() {
        let mut m = sh_machine();
        m.feed_str(InnerKind::Item, "x=5").unwrap();
        let snap = m.snapshot().unwrap();
        m.feed_str(InnerKind::Item, "x=9").unwrap();
        assert_eq!(
            m.feed_str(InnerKind::Expr, "x").unwrap().value.as_deref(),
            Some("9")
        );
        m.restore(&snap).unwrap();
        assert_eq!(
            m.feed_str(InnerKind::Expr, "x").unwrap().value.as_deref(),
            Some("5")
        );
    }

    /// A machine without a `type_wrap` refuses the typing judgment with an
    /// actionable message rather than a broken query.
    #[test]
    fn no_type_wrap_is_an_honest_error() {
        let mut m = sh_machine();
        let err = m.type_of_str("1 + 1").unwrap_err();
        assert!(err.to_string().contains("typing judgment"), "got: {err}");
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

    /// The typing judgment answers the type's spelling, and sees the fed
    /// definitions like any other query.
    #[test]
    fn python_answers_types() {
        let spec = MachineSpec {
            type_wrap: Some("print(type({}).__name__)".into()),
            ..MachineSpec::from_hashbang("#!/usr/bin/env python3", "print(repr({}))", ".py")
                .unwrap()
        };
        let mut m = ScriptMachine::new("py", spec);
        assert_eq!(
            m.type_of_str("21 + 21").unwrap().value.as_deref(),
            Some("int")
        );
        m.feed_str(InnerKind::Item, "s = 'hi'").unwrap();
        assert_eq!(
            m.type_of_str("s * 2").unwrap().value.as_deref(),
            Some("str")
        );
    }
}
