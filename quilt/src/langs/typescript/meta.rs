use miette::Result;

use super::ops::{build_quote_code, build_tuple_code, build_unquote_code, build_variadic_block};
use crate::lang::Arity;
use crate::prelude::{Index, *};
use crate::{
    meta::{MetaLanguage, Prelude},
    qterm::QTerm,
    term::CmdOrHole,
};

/**************************************************************/

#[derive(Default)]
pub struct TypeScriptMetaLanguage;

impl MetaLanguage for TypeScriptMetaLanguage {
    fn expand_quote(
        &self,
        _lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        Ok(build_quote_code(tag, i, lang2, qterm, cmds))
    }

    fn expand_unquote(
        &self,
        _lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        Ok(build_unquote_code(tag, i, lang2, qterm, cmds))
    }

    fn expand_tuple(
        &self,
        _lang1: &str,
        tag: &str,
        qterms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
        arity: Arity,
    ) -> Result<Arc<QTerm>> {
        Ok(if arity == Arity::Variadic {
            build_variadic_block(tag, cmds, qterms)
        } else {
            build_tuple_code(tag, cmds, qterms)
        })
    }

    // `↑` is target-directed and written prefix, `↑(value)`: into TypeScript it
    // spells the `qlift` function (number/string/boolean → TS literals); into
    // HTML it spells `qlift_html`, which entity-escapes lifted strings at
    // runtime. Both live in the `quilt-wasm` runtime.
    fn lift_str(&self, target: &str) -> Result<&'static str> {
        match target {
            "typescript" | "ts" => Ok("qlift"),
            "html" => Ok("qlift_html"),
            _ => miette::bail!("typescript can't lift into {target:?}: no spelling registered"),
        }
    }

    fn reduce_str(&self, target: &str) -> Result<&'static str> {
        match target {
            "" | "typescript" | "ts" => Ok("reduce()"),
            _ => miette::bail!("typescript can't reduce via {target:?}: no spelling registered"),
        }
    }

    fn name_str(&self) -> Result<&'static str> {
        Ok("name")
    }

    fn type_str(&self) -> Result<&'static str> {
        Ok("QTerm")
    }

    /// No spelling: `←` needs a named `b_` accumulator in scope, and this host
    /// has none (issue #152). Same shape as Python's — see
    /// [`PythonMetaLanguage::emit_str`](crate::langs::python::meta::PythonMetaLanguage::emit_str)
    /// for the full reasoning.
    ///
    /// TypeScript *could* host an accumulator, since an IIFE gives it
    /// statements in expression position — but `WasmBuilder::e` consumes and
    /// returns the builder rather than mutating it, and `WasmQTerm` exposes no
    /// `emit` method, so the runtime half does not exist either. Until it does,
    /// a ground `←` fails here rather than expanding to generated TypeScript
    /// that references an undefined `b_`.
    ///
    /// The working alternative is the same: build the sequence with your own
    /// builder in ground code and splice the finished term with `↙…↘`.
    ///
    /// ```ts
    /// let b = tb("statement_block");
    /// for (const n of names) b = b.e(ts↖console.log(↙name(n)↘)↗);
    /// const body = b.b();
    /// ```
    fn emit_str(&self) -> Result<&'static str> {
        miette::bail!(
            "typescript can't emit `←`: the fluent `.e(child)` chain has no named `b_` \
             accumulator to emit into — build the sequence with your own `tb(..)` builder in \
             ground code and splice the finished term with `↙…↘`"
        )
    }

    /// The `import { … } from "quilt"` an expanded file opens with (issue
    /// #274) — bound by `quilt run` to `quilt-wasm/node`, and in the browser
    /// demos by an import map to `examples/web/quilt-rt.js`.
    ///
    /// This is the one host where the prelude is not a constant, and the reason
    /// the hook takes `targets` at all: TypeScript has no glob import, so the
    /// list has to name every runtime function the expansion calls — and
    /// [`lift_str`](Self::lift_str) is target-directed, so quoting a *new*
    /// language adds a name to it. Written by hand that is boilerplate you can
    /// get wrong in a way the source does not hint at: a file that starts
    /// quoting HTML needs `qlift_html` added, and finds out at run time.
    ///
    /// Unknown targets are skipped rather than reported: a language with no
    /// lift spelling here fails at the `↑` itself, with a message about that
    /// glyph, which is a better place to hear it than a prelude.
    fn prelude(&self, targets: &[&str]) -> Option<Prelude> {
        // The builder vocabulary every expansion uses, in the order the
        // hand-written imports have always listed it, with the lift spellings
        // slotted in where `qlift` sat.
        const HEAD: [&str; 9] = [
            "tb", "leaf", "sym", "quote", "unquote", "cmd", "write", "push", "name",
        ];
        const TAIL: [&str; 3] = ["NL", "POP", "HOLE"];
        let mut names: Vec<&str> = HEAD.to_vec();
        for target in targets {
            if let Ok(lift) = self.lift_str(target) {
                if !names.contains(&lift) {
                    names.push(lift);
                }
            }
        }
        names.extend_from_slice(&TAIL);
        Some(Prelude::new(
            format!("import {{ {} }} from \"quilt\";", names.join(", ")),
            // Both spellings of the specifier, because a duplicate *named*
            // import is a hard `Duplicate identifier` error here — unlike the
            // glob imports the other hosts inject, which double harmlessly.
            ["from \"quilt\"", "from 'quilt'"],
        ))
    }
}
