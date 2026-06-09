use crate::prelude::*;
use crate::{
    strcmd::{PrefixWriter, StrCmd},
    validate::Validate,
};
use miette::IntoDiagnostic;
use serde::{Deserialize, Serialize};
use std::{io::Write, sync::Arc};

/**************************************************************/

pub trait Term: Sized {
    type Tag;

    fn tag(&self) -> Self::Tag;
    fn children(&self) -> impl Iterator<Item = &Self>;
    fn len(&self) -> usize {
        self.children().count()
    }
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn get(&self, i: usize) -> Option<&Self> {
        self.children().nth(i)
    }
}

/**************************************************************/

pub struct ArcTerm<T: Clone> {
    tag: T,
    children: Box<[Arc<Self>]>,
}

impl<T: Clone> Term for ArcTerm<T> {
    type Tag = T;

    fn tag(&self) -> Self::Tag {
        self.tag.clone()
    }

    fn children(&self) -> impl Iterator<Item = &Self> {
        self.children.iter().map(|x| x.as_ref())
    }

    fn len(&self) -> usize {
        self.children.len()
    }

    fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

impl<T: Clone> Term for Arc<ArcTerm<T>> {
    type Tag = T;

    fn tag(&self) -> Self::Tag {
        self.tag.clone()
    }

    fn children(&self) -> impl Iterator<Item = &Self> {
        self.children.iter()
    }

    fn len(&self) -> usize {
        self.children.len()
    }

    fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

/**************************************************************/

pub trait STerm: Term {
    fn write<W: Write>(&self, writer: &mut PrefixWriter<'_, W>);

    fn coparse(&self) -> String {
        let mut buf = Vec::new();
        let mut writer = PrefixWriter::new(&mut buf);
        self.write(&mut writer);
        String::from_utf8(buf).unwrap()
    }

    fn dump(&self, filename: &str) -> Result<()> {
        let mut file = std::fs::File::create(filename).into_diagnostic()?;
        let mut writer = PrefixWriter::new(&mut file);
        self.write(&mut writer);
        Ok(())
    }

    fn dump_with_cmds(&self, filename: &str, prefix: &[StrCmd], suffix: &[StrCmd]) -> Result<()> {
        let mut file = std::fs::File::create(filename).into_diagnostic()?;
        let mut writer = PrefixWriter::new(&mut file);
        for cmd in prefix {
            writer.interpret(cmd);
        }
        self.write(&mut writer);
        for cmd in suffix {
            writer.interpret(cmd);
        }
        Ok(())
    }
}

/**************************************************************/

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CmdOrHole {
    Cmd(StrCmd),
    Hole,
}

pub fn cmd(cmd: StrCmd) -> CmdOrHole {
    CmdOrHole::Cmd(cmd)
}
pub fn hole() -> CmdOrHole {
    CmdOrHole::Hole
}

pub const HOLE: CmdOrHole = CmdOrHole::Hole;

#[derive(Debug, Clone)]
pub struct ArcSTerm<T> {
    tag: T,
    children: Box<[Arc<Self>]>,
    cmds: Box<[CmdOrHole]>,
}

impl<T: Clone> Term for ArcSTerm<T> {
    type Tag = T;

    fn tag(&self) -> Self::Tag {
        self.tag.clone()
    }

    fn children(&self) -> impl Iterator<Item = &Self> {
        self.children.iter().map(|x| x.as_ref())
    }

    fn len(&self) -> usize {
        self.children.len()
    }

    fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

impl<T: Clone> Term for Arc<ArcSTerm<T>> {
    type Tag = T;

    fn tag(&self) -> Self::Tag {
        self.tag.clone()
    }

    fn children(&self) -> impl Iterator<Item = &Self> {
        self.children.iter()
    }

    fn len(&self) -> usize {
        self.children.len()
    }

    fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

impl<T: Clone> STerm for ArcSTerm<T> {
    fn write<W: Write>(&self, writer: &mut PrefixWriter<'_, W>) {
        let mut children = self.children.iter();
        for cmd in &self.cmds {
            match cmd {
                CmdOrHole::Cmd(cmd) => writer.interpret(cmd),
                CmdOrHole::Hole => children.next().unwrap().write(writer),
            }
        }
    }
}

impl<T> Validate for ArcSTerm<T> {
    type Error = miette::Error;

    fn validate(self) -> Result<Self, Self::Error> {
        let mut depth: u32 = 0;
        let mut holes = 0;
        for cmd in &self.cmds {
            match cmd {
                CmdOrHole::Cmd(cmd) => match cmd {
                    StrCmd::Push(_) => depth += 1,
                    StrCmd::Pop => {
                        if depth == 0 {
                            miette::bail!("running stack depth below 0");
                        }
                        depth -= 1;
                    }
                    _ => (),
                },
                CmdOrHole::Hole => {
                    holes += 1;
                }
            }
        }
        if depth != 0 {
            miette::bail!("total stack depth not 0");
        }
        if holes != self.children.len() {
            miette::bail!("number of holes does not match number of children");
        }
        Ok(self)
    }
}

pub struct ArcSTermBuilder<T> {
    tag: T,
    children: Vec<Arc<ArcSTerm<T>>>,
    cmds: Vec<CmdOrHole>,
}

impl<T> ArcSTermBuilder<T> {
    pub fn new(tag: T) -> Self {
        Self {
            tag,
            children: Vec::new(),
            cmds: Vec::new(),
        }
    }

    pub fn child(mut self, child: &Arc<ArcSTerm<T>>) -> Self {
        self.cmds.push(CmdOrHole::Hole);
        self.children.push(child.clone());
        self
    }

    pub fn cmd(mut self, cmd: StrCmd) -> Self {
        self.cmds.push(CmdOrHole::Cmd(cmd));
        self
    }

    pub fn build(self) -> ArcSTerm<T> {
        ArcSTerm {
            tag: self.tag,
            children: self.children.into_boxed_slice(),
            cmds: self.cmds.into_boxed_slice(),
        }
    }

    // convenience methods

    pub fn write(self, s: &str) -> Self {
        self.cmd(StrCmd::Write(s.into()))
    }

    pub fn newline(self) -> Self {
        self.cmd(StrCmd::NewLine)
    }

    pub fn push(self, s: &str) -> Self {
        self.cmd(StrCmd::Push(s.into()))
    }

    pub fn pop(self) -> Self {
        self.cmd(StrCmd::Pop)
    }

    pub fn c(self, child: &Arc<ArcSTerm<T>>) -> Self {
        self.child(child)
    }

    pub fn w(self, s: &str) -> Self {
        self.write(s)
    }

    pub fn n(self) -> Self {
        self.newline()
    }

    pub fn p(self, s: &str) -> Self {
        self.push(s)
    }

    pub fn x(self) -> Self {
        self.pop()
    }

    pub fn b(self) -> ArcSTerm<T> {
        self.build()
    }
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sterm() -> miette::Result<()> {
        let sterm = ArcSTermBuilder::new(()).w("foo").b().v()?;
        let sterm = arc(sterm);
        let sterm = ArcSTermBuilder::new(())
            .c(&sterm)
            .p(">>> ")
            .n()
            .c(&sterm)
            .x()
            .n()
            .c(&sterm)
            .b()
            .v()?;

        let mut buf = std::io::BufWriter::new(Vec::new());
        let mut writer = PrefixWriter::new(&mut buf);
        sterm.write(&mut writer);
        let bytes = buf.into_inner().unwrap();
        let s = String::from_utf8(bytes).unwrap();
        println!("{s}");

        Ok(())
    }
}
