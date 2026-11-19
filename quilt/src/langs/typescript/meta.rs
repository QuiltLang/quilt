use miette::Result;

use super::ops::{
    build_qt_code, build_quote_code, build_str_code, build_tuple_code, build_unquote_code,
    build_variadic_block,
};
use crate::lang::Arity;
use crate::prelude::{Index, *};
use crate::{
    meta::{MetaLanguage, Piece, Prelude},
    qterm::{QTerm, Repr},
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
    // runtime, and into text `qlift_text`. All live in the `quilt-wasm`
    // runtime.
    fn lift_str(&self, target: &str) -> Result<&'static str> {
        match target {
            "typescript" | "ts" => Ok("qlift"),
            "html" => Ok("qlift_html"),
            // Into text: `qlift_text`, or `qlift_text_in_quotes` for a hole the
            // author wrapped in `"…"` (`lift::text`). Both take an optional
            // dialect object as a second argument.
            "text" | "txt" => Ok("qlift_text"),
            crate::lift::TEXT_IN_QUOTES => Ok("qlift_text_in_quotes"),
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

    /// `←` is *functional* here, as in Nix: written prefix with its argument
    /// in parentheses, `←(items)` or `←(items, sep)`, it spells the runtime's
    /// `qseq`, which turns a whole iterable of fragments into one term that
    /// splices them in order — a newline between each by default, so a
    /// sequence of statements spliced into a block stays one per line at the
    /// block's indent:
    ///
    /// ```ts
    /// const body = ts↖{
    ///   ↙←(names.map((n) => ts↖console.log(↙↑(n)↘);↗))↘
    /// }↗;
    /// ```
    ///
    /// Rust's imperative reading (append to a `b_` accumulator once around a
    /// loop, issue #152) would need statements in expression position and a
    /// mutable builder; the functional one needs neither, and is what
    /// TypeScript code builds lists with anyway.
    fn emit_str(&self) -> Result<&'static str> {
        Ok("qseq")
    }

    /// Inside a `⟨str⟩` quote a lifted value must be text: into TypeScript
    /// `↑` spells `qlift_str`, the literal's source as a string (a safe
    /// integer goes straight through `String`). Into another language the
    /// term-building lift still works, since a term interpolates as its
    /// source. `⟨cst⟩` and `⟨qt⟩` quotes take terms, so they lift as before.
    fn lift_str_in(&self, target: &str, repr: Repr) -> Result<&'static str> {
        match (repr, target) {
            (Repr::Str, "typescript" | "ts") => Ok("qlift_str"),
            _ => self.lift_str(target),
        }
    }

    /// `←(items)` joins the items into one string inside a `⟨str⟩` quote
    /// (`qjoin`), and builds a `qseq` term everywhere else.
    fn emit_str_in(&self, repr: Repr) -> Result<&'static str> {
        match repr {
            Repr::Str => Ok("qjoin"),
            Repr::Cst | Repr::Qt => self.emit_str(),
        }
    }

    /// `⟨str⟩`: a template literal; `⟨qt⟩`: a flat builder chain. See
    /// [`build_str_code`] and [`build_qt_code`].
    fn expand_flat(&self, repr: Repr, _lang2: &str, pieces: &[Piece]) -> Result<Arc<QTerm>> {
        Ok(match repr {
            Repr::Str => build_str_code(pieces),
            Repr::Qt => build_qt_code(pieces),
            Repr::Cst => miette::bail!("a `⟨cst⟩` quote is not flattened"),
        })
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
        const HEAD: [&str; 12] = [
            "tb",
            "leaf",
            "sym",
            "quote",
            "unquote",
            "cmd",
            "write",
            "push",
            "name",
            "qseq",
            "qjoin",
            "qlift_str",
        ];
        const TAIL: [&str; 3] = ["NL", "POP", "HOLE"];
        let mut names: Vec<&str> = HEAD.to_vec();
        for target in targets {
            // A text quote can hold both spellings: the expander picks the
            // in-quotes one per hole, from the characters around it, so the
            // target list (languages, not positions) never names it.
            let positions: &[&str] = if matches!(*target, "text" | "txt") {
                &[target, crate::lift::TEXT_IN_QUOTES]
            } else {
                &[target]
            };
            for position in positions {
                if let Ok(lift) = self.lift_str(position) {
                    if !names.contains(&lift) {
                        names.push(lift);
                    }
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
