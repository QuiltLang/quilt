use serde::{Deserialize, Serialize};

/**************************************************************/

/// A single string command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrCmd {
    /// Write a str. Any newlines will ignore the prefix.
    Write(Box<str>),
    /// Add a newline, respecting the prefix.
    NewLine,
    /// Add a prefix to the stack.
    Push(Box<str>),
    /// Pop a prefix from the stack.
    Pop,
}

pub struct PrefixWriter<'a, W: std::io::Write> {
    file: &'a mut W,
    stack: Vec<Box<str>>,
    /// Whether what is being written is `.quilt` *source* rather than target
    /// output, and so must have its Quilt glyphs `\`-escaped. Off by default:
    /// the common job is rendering an expanded term as the code it generates,
    /// where a glyph is ordinary text in the target language and escaping it
    /// would corrupt the file. See [`crate::term::STerm::coparse_quilt`] (#223).
    escaping: bool,
    /// Whether the text being written sits inside a `↖…↗` / `↙…↘` bracket. See
    /// [`Self::set_bracketed`].
    bracketed: bool,
}

impl<'a, W: std::io::Write> PrefixWriter<'a, W> {
    pub fn new(file: &'a mut W) -> Self {
        Self {
            file,
            stack: Vec::new(),
            escaping: false,
            bracketed: false,
        }
    }

    /// A writer that renders `.quilt` source: glyphs in content are escaped.
    pub fn quilt(file: &'a mut W) -> Self {
        Self {
            escaping: true,
            ..Self::new(file)
        }
    }

    /// Set escaping, returning what it was — so a caller about to write Quilt's
    /// own syntax (the arrows around a quote, a deferred operator) can suspend
    /// it and put it back.
    pub fn set_escaping(&mut self, escaping: bool) -> bool {
        std::mem::replace(&mut self.escaping, escaping)
    }

    #[must_use]
    pub fn escaping(&self) -> bool {
        self.escaping
    }

    /// Set "inside `↖…↗` / `↙…↘`", returning what it was.
    ///
    /// Only a bracketed fragment can hold a *deferred* operator — one the
    /// expander left for a later stage — so this is what keeps the exemption
    /// for those from reaching ground level, where every `↑ ↓ ←` has already
    /// been spelled out and a bare glyph can only be escaped content.
    pub fn set_bracketed(&mut self, bracketed: bool) -> bool {
        std::mem::replace(&mut self.bracketed, bracketed)
    }

    #[must_use]
    pub fn bracketed(&self) -> bool {
        self.bracketed
    }

    pub fn write(&mut self, s: &str) {
        if self.escaping {
            write!(self.file, "{}", crate::glyphs::escape(s)).unwrap();
        } else {
            write!(self.file, "{s}").unwrap();
        }
    }

    pub fn newline(&mut self) {
        writeln!(self.file).unwrap();
        for prefix in &self.stack {
            write!(self.file, "{prefix}").unwrap();
        }
    }

    pub fn push(&mut self, s: &str) {
        self.stack.push(s.into());
    }

    pub fn pop(&mut self) {
        self.stack.pop();
    }

    pub fn interpret(&mut self, cmd: &StrCmd) {
        match cmd {
            StrCmd::Write(s) => self.write(s),
            StrCmd::NewLine => self.newline(),
            StrCmd::Push(s) => self.push(s),
            StrCmd::Pop => self.pop(),
        }
    }
}

pub fn write(s: &str) -> StrCmd {
    StrCmd::Write(s.into())
}

pub fn newline() -> StrCmd {
    StrCmd::NewLine
}

pub fn push(s: &str) -> StrCmd {
    StrCmd::Push(s.into())
}

pub fn pop() -> StrCmd {
    StrCmd::Pop
}

pub const NL: StrCmd = StrCmd::NewLine;
pub const POP: StrCmd = StrCmd::Pop;
