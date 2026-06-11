//! String-style lifting used by bootstrap meta-language.

use crate::{prelude::*, term::CmdOrHole};

/**************************************************************/

pub trait StrLift {
    /// Lift this to a string that reduces to it.
    fn strlift(&self) -> String;
}

impl StrLift for Arc<QTerm> {
    fn strlift(&self) -> String {
        match &**self {
            // span is dropped: lifted code rebuilds the term without one
            QTerm::Quote {
                tag,
                index,
                lang,
                term,
                cmds,
                ..
            } => format!(
                "quote({}, {}, {}, {}, &{})",
                (**tag).strlift(),
                index.strlift(),
                (**lang).strlift(),
                (*term).strlift(),
                (**cmds).strlift(),
            ),
            QTerm::Unquote {
                tag,
                index,
                lang,
                term,
                cmds,
                ..
            } => format!(
                "unquote({}, {}, {}, {}, &{})",
                (**tag).strlift(),
                index.strlift(),
                (**lang).strlift(),
                (*term).strlift(),
                (**cmds).strlift(),
            ),
            QTerm::Tuple { tag, terms, cmds } => {
                // use shorthands when possible
                if terms.is_empty() && cmds.len() == 1 {
                    if let CmdOrHole::Cmd(StrCmd::Write(code)) = &cmds[0] {
                        return if tag == code {
                            format!("sym({})", (**tag).strlift())
                        } else {
                            format!("leaf({}, {})", (**tag).strlift(), (**code).strlift())
                        };
                    }
                }
                // fall back to full representation
                let mut children = terms.iter();
                let mut ret = format!("tb({})", (**tag).strlift());
                for cmd in cmds {
                    let s = match cmd {
                        CmdOrHole::Cmd(StrCmd::Write(s)) => &format!(".w({})", (**s).strlift()),
                        CmdOrHole::Cmd(StrCmd::NewLine) => ".n()",
                        CmdOrHole::Cmd(StrCmd::Push(s)) => &format!(".p({})", (**s).strlift()),
                        CmdOrHole::Cmd(StrCmd::Pop) => ".x()",
                        CmdOrHole::Hole => &format!(".c(&{})", children.next().unwrap().strlift()),
                    };
                    ret.push_str(s);
                }
                ret.push_str(".b()");
                ret
            }
        }
    }
}

impl StrLift for u8 {
    fn strlift(&self) -> String {
        format!("{self}")
    }
}

impl<T: StrLift> StrLift for Box<T> {
    fn strlift(&self) -> String {
        format!("bx({})", (**self).strlift())
    }
}

impl<T: StrLift> StrLift for Vec<T> {
    fn strlift(&self) -> String {
        format!(
            "vec![{}]",
            self.iter()
                .map(|x| x.strlift())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl<T: StrLift> StrLift for [T] {
    fn strlift(&self) -> String {
        format!(
            "[{}]",
            self.iter()
                .map(|x| x.strlift())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

// why is this necessary given impls for Box<T> and [T]?
impl<T: StrLift> StrLift for Box<[T]> {
    fn strlift(&self) -> String {
        format!("bx({})", (**self).strlift())
    }
}

// why is this necessary given impls for Box<T> and str?
impl StrLift for Box<str> {
    fn strlift(&self) -> String {
        format!("bx({})", (**self).strlift())
    }
}

impl<T: StrLift, const N: usize> StrLift for [T; N] {
    fn strlift(&self) -> String {
        format!(
            "[{}]",
            self.iter()
                .map(|x| x.strlift())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl StrLift for StrCmd {
    fn strlift(&self) -> String {
        match self {
            StrCmd::Write(s) => format!("write({})", (**s).strlift()),
            StrCmd::NewLine => "NL".to_string(),
            StrCmd::Push(s) => format!("push({})", (**s).strlift()),
            StrCmd::Pop => "POP".to_string(),
        }
    }
}

impl StrLift for str {
    fn strlift(&self) -> String {
        let escaped = self.replace('\\', "\\\\").replace('\"', "\\\"");
        format!("\"{escaped}\"")
    }
}

impl StrLift for &str {
    fn strlift(&self) -> String {
        format!("\"{self}\"")
    }
}

impl StrLift for char {
    fn strlift(&self) -> String {
        format!("'{self}'")
    }
}

impl StrLift for i32 {
    fn strlift(&self) -> String {
        format!("{self}")
    }
}

impl<T: StrLift> StrLift for Arc<T> {
    fn strlift(&self) -> String {
        format!("arc({})", (**self).strlift())
    }
}

impl StrLift for CmdOrHole {
    fn strlift(&self) -> String {
        match self {
            CmdOrHole::Cmd(cmd) => format!("cmd({})", cmd.strlift()),
            CmdOrHole::Hole => "HOLE".to_string(),
        }
    }
}

impl StrLift for String {
    fn strlift(&self) -> String {
        format!("\"{}\"", self.replace('\"', "\\\""))
    }
}

impl StrLift for usize {
    fn strlift(&self) -> String {
        format!("{self}")
    }
}
