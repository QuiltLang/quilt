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
}

impl<'a, W: std::io::Write> PrefixWriter<'a, W> {
    pub fn new(file: &'a mut W) -> Self {
        let stack = Vec::new();
        Self { file, stack }
    }

    pub fn write(&mut self, s: &str) {
        write!(self.file, "{s}").unwrap();
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
